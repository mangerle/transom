use super::{
    CompletionItem, CompletionKind, CompletionPool, MAX_MATCHES, MAX_NAME_CHARS, MIN_TOKEN_CHARS,
    TokenSlot, complete, echo_lagging, fuzzy_matches, history_items, last_token, merged_command,
    token_slot,
};

fn item(name: &str, command: &str, note: &str) -> CompletionItem {
    CompletionItem::new(
        name.to_string(),
        command.to_string(),
        note.to_string(),
        CompletionKind::Command,
    )
}

fn sample() -> Vec<CompletionItem> {
    vec![
        item(
            "系统更新",
            "sudo apt update && sudo apt upgrade -y",
            "更新索引并升级",
        ),
        item(
            "安装软件包",
            "sudo apt install <包名>",
            "把包名换成要装的软件",
        ),
        item("监听端口", "sudo ss -tlnp", "列出监听端口"),
        item("磁盘余量", "df -h", "按挂载点列出使用率"),
        item(
            "Docker 容器",
            "docker ps -a --format 'table {{.Names}}'",
            "列出全部容器",
        ),
    ]
}

fn names(hits: &[CompletionItem]) -> Vec<String> {
    hits.iter().map(|hit| hit.name.clone()).collect()
}

#[test]
fn short_token_shows_nothing() {
    let items = sample();
    assert!(fuzzy_matches(&items, "").is_empty());
    assert!(
        fuzzy_matches(&items, "a".repeat(MIN_TOKEN_CHARS - 1).as_str()).is_empty(),
        "命令位一两个字符就弹太吵"
    );
    assert!(!fuzzy_matches(&items, "ap").is_empty());
}

#[test]
fn command_word_prefix_wins_over_note() {
    let items = sample();
    let hits = fuzzy_matches(&items, "apt");
    assert_eq!(hits.len(), 2);
    // 「安装软件包」与「系统更新」都在命令里出现 apt，按名称稳定排序。
    let names: Vec<&str> = hits.iter().map(|&i| items[i].name.as_str()).collect();
    assert_eq!(names, ["安装软件包", "系统更新"]);
}

#[test]
fn name_and_note_are_searchable() {
    let items = sample();
    assert_eq!(fuzzy_matches(&items, "磁盘"), vec![3]);
    // 备注里也有「端口」，仍应命中同名条目。
    assert_eq!(fuzzy_matches(&items, "端口"), vec![2]);
    assert_eq!(fuzzy_matches(&items, "docker"), vec![4]);
}

#[test]
fn limit_is_capped() {
    let items: Vec<CompletionItem> = (0..32)
        .map(|n| item(&format!("命令 {n:02}"), "echo hi", ""))
        .collect();
    assert_eq!(fuzzy_matches(&items, "命令").len(), MAX_MATCHES);
}

#[test]
fn slot_follows_the_command_segment() {
    assert_eq!(token_slot(""), TokenSlot::Command);
    assert_eq!(token_slot("cd "), TokenSlot::Argument);
    assert_eq!(token_slot("sudo apt "), TokenSlot::Argument);
    // 分隔符之后是新的命令段，段首又是命令位。
    assert_eq!(token_slot("ls -la | "), TokenSlot::Command);
    assert_eq!(token_slot("cd /tmp && cd "), TokenSlot::Argument);
    assert_eq!(token_slot("echo hi;"), TokenSlot::Command);
}

#[test]
fn wrappers_do_not_make_the_next_word_an_argument() {
    // 段首只剩包装词时，正在输入的词仍是命令位：`sudo dock` 要能捞回
    // 库里的 docker 命令，而不是只匹配以 sudo 开头的写法。
    assert_eq!(token_slot("sudo "), TokenSlot::Command);
    assert_eq!(token_slot("sudo nohup "), TokenSlot::Command);
    // 包装词之后还有实义词，回到参数位（`sudo apt ` 接续写不受影响）。
    assert_eq!(token_slot("sudo apt "), TokenSlot::Argument);
    // `cd sudo` 里的 sudo 是 cd 的参数，不是包装词：剥词只从段首起。
    assert_eq!(token_slot("cd sudo "), TokenSlot::Argument);

    let items = sample();
    assert!(names(&complete(&items, "dock", "sudo ")).contains(&"Docker 容器".to_string()));
    // 命令位候选若本身就带 sudo，接受补全时前缀重叠会被 merged_command 合并，
    // 不会补成 `sudo sudo ss -tlnp`。
    let merged = merged_command("sudo ", "sudo ss -tlnp");
    assert_eq!(merged, "ss -tlnp");
}

#[test]
fn argument_position_does_not_suggest_other_commands() {
    let items = vec![
        item("进入目录", "cd /var/log", "去看系统日志"),
        item(
            "按名查找",
            "sudo find / -name '<文件名>' 2>/dev/null",
            "全盘找文件",
        ),
    ];
    // `cd find`：`find` 是 cd 的参数，不该因为片段里有 `sudo find …` 就命中。
    assert!(complete(&items, "find", "cd ").is_empty());
    // 同一个词落在命令位，照旧整库搜索。
    assert_eq!(names(&complete(&items, "find", "")), ["按名查找"]);
    // 参数位也不许拿参数去撞别的命令名（`cd docker` 不等于要找 docker 命令）。
    assert!(complete(&items, "docker", "cd ").is_empty());
}

#[test]
fn argument_position_only_takes_aligned_continuation() {
    let items = sample();
    // 段末尾正好是候选命令的开头：`sudo apt up` → `sudo apt update …`。
    assert_eq!(names(&complete(&items, "up", "sudo apt ")), ["系统更新"]);
    // 只对齐一个词的 `apt up` 接不上以 sudo 开头的片段。
    assert!(complete(&items, "up", "apt ").is_empty());
    // 命令位仍会把含 `in` 的条目排前面。
    let with_powershell = vec![
        item(
            "下载文件",
            "Invoke-WebRequest -Uri <地址> -OutFile <文件名>",
            "下载文件到本机",
        ),
        item("安装软件包", "sudo apt install <包名>", ""),
    ];
    assert_eq!(names(&complete(&with_powershell, "in", ""))[0], "下载文件");
    assert_eq!(
        names(&complete(&with_powershell, "in", "sudo apt "))[0],
        "安装软件包"
    );
}

#[test]
fn empty_token_at_argument_lists_whats_next() {
    let items = sample();
    // 刚敲完 `sudo apt `：列出这条命令后面能接的写法。
    assert_eq!(
        names(&complete(&items, "", "sudo apt ")),
        ["系统更新", "安装软件包"]
    );
    // 没有接得上的命令就什么都不列。
    assert!(complete(&items, "", "cd ").is_empty());
}

#[test]
fn subcommand_continues_its_command() {
    let items = vec![
        item("提交改动", "git commit -m '<说明>'", "提交到当前分支"),
        item("查看改动", "git status", "看工作区状态"),
        item("添加远程", "git remote add <名> <地址>", "接远程仓库"),
    ];
    assert_eq!(names(&complete(&items, "re", "git ")), ["添加远程"]);
    assert_eq!(names(&complete(&items, "s", "git ")), ["查看改动"]);
    // 子命令之后的参数不再去撞别的 git 命令：`git commit msg` 什么都不提示。
    assert!(complete(&items, "msg", "git commit ").is_empty());
}

#[test]
fn dash_offers_options_of_the_command_in_position() {
    let items = vec![
        item("提交改动", "git commit -m '<说明>'", ""),
        item("打标签", "git tag -a -m '<说明>'", ""),
        item("改写上次提交", "git commit --amend", ""),
        item("忽略大小写比较", "git diff -i --no-index", ""),
    ];
    // 用得多的排前面，同频按字母；来源命令留在 note 里给浮层显示。
    let hits = complete(&items, "-", "git ");
    assert_eq!(
        hits.iter()
            .map(|hit| hit.command.as_str())
            .collect::<Vec<_>>(),
        ["-m", "--amend", "--no-index", "-a", "-i"]
    );
    assert_eq!(hits[0].note, "git commit -m '<说明>'");
    assert_eq!(hits[0].kind, CompletionKind::Flag);
    // `--` 只留长选项。
    assert_eq!(
        complete(&items, "--", "git ")
            .iter()
            .map(|hit| hit.command.as_str())
            .collect::<Vec<_>>(),
        ["--amend", "--no-index"]
    );
    // 命令对不上就没有选项可给。
    assert!(complete(&items, "-", "cd ").is_empty());
    // 单独的 `-` 与 `--` 本身不是选项。
    let bare = vec![item("过滤", "grep - pattern", "")];
    assert!(complete(&bare, "-", "grep ").is_empty());
}

fn history(entries: &[&str]) -> Vec<CompletionItem> {
    let entries: Vec<String> = entries.iter().map(|entry| (*entry).to_string()).collect();
    history_items(&entries, &[])
}

#[test]
fn history_lines_rank_newest_first_and_beat_snippets() {
    let recent_first = [
        &history(&["git commit -m '修复登录崩溃'", "git status"])[..],
        &sample()[..],
    ]
    .concat();
    let hits = complete(&recent_first, "git", "");
    assert_eq!(hits[0].kind, CompletionKind::History);
    assert_eq!(hits[0].command, "git commit -m '修复登录崩溃'");
    assert_eq!(hits[1].command, "git status");
    // 历史顺序反过来，候选顺序跟着变：靠的是新旧次序，不是名称字母序。
    let older_first = history(&["git status", "git commit -m '修复登录崩溃'"]);
    let hits = complete(&older_first, "git", "");
    assert_eq!(hits[0].command, "git status");
}

#[test]
fn history_line_matches_on_any_word_of_the_line() {
    let items = history(&["git commit -m '修复登录崩溃'"]);
    // 中间词、乃至注释里的字都在候选一侧参与匹配。
    let hits = complete(&items, "登录", "");
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].command, "git commit -m '修复登录崩溃'");
    // 整条命令照旧写回终端（前缀已有的词不重复）。
    assert_eq!(
        merged_command("", &hits[0].command),
        "git commit -m '修复登录崩溃'"
    );
}

#[test]
fn history_continues_the_argument_position() {
    let items = history(&["git remote add origin git@example.com:repo.git"]);
    let hits = complete(&items, "ad", "git remote ");
    assert_eq!(hits.len(), 1, "参数位要接得上上次那条");
    assert_eq!(hits[0].kind, CompletionKind::History);
    assert_eq!(
        merged_command("git remote ", &hits[0].command),
        "add origin git@example.com:repo.git"
    );
    // 接不上的参数位仍什么都不给。
    assert!(complete(&items, "ad", "cd ").is_empty());
    // 参数位输入 `-`：选项取自上次那条命令，浮层第二列显示它来自哪条。
    let flagged = history(&["git commit -m '修复登录崩溃'"]);
    let hits = complete(&flagged, "-", "git commit ");
    assert_eq!(hits[0].command, "-m");
    assert_eq!(hits[0].note, "git commit -m '修复登录崩溃'");
}

#[test]
fn snippet_command_already_saved_is_not_shown_twice() {
    let presets = vec![item("磁盘余量", "df -h", "按挂载点列出使用率")];
    let entries: Vec<String> = ["df -h", "df -i"]
        .iter()
        .map(|entry| (*entry).to_string())
        .collect();
    let hits = history_items(&entries, &presets);
    assert_eq!(
        hits.iter()
            .map(|hit| hit.command.as_str())
            .collect::<Vec<_>>(),
        ["df -i"]
    );
}

#[test]
fn long_history_line_only_truncates_its_display_name() {
    let long = format!("echo {}", "x".repeat(300));
    let entries = vec![long.clone()];
    let hits = history_items(&entries, &[]);
    assert!(hits[0].name.chars().count() <= MAX_NAME_CHARS + 1);
    assert!(hits[0].name.ends_with('…'));
    assert_eq!(hits[0].command, long);
}

#[test]
fn history_and_snippets_share_the_argument_slot() {
    let history = history(&["git commit -m 'fix login crash'", "git commit --amend"]);
    let presets = sample();
    let pool = [history.as_slice(), presets.as_slice()].concat();
    // 刚敲完 `git commit `：最近用过的那条接在最前，补的是它剩下的部分。
    let hits = complete(&pool, "", "git commit ");
    assert_eq!(hits.len(), 2);
    assert_eq!(hits[0].kind, CompletionKind::History);
    assert_eq!(
        merged_command("git commit ", &hits[0].command),
        "-m 'fix login crash'"
    );
    // 候选总数仍受 MAX_MATCHES 约束。
    let chatty: Vec<String> = (0..12).map(|n| format!("echo step {n}")).collect();
    let loud = [history_items(&chatty, &[]).as_slice(), presets.as_slice()].concat();
    assert_eq!(complete(&loud, "echo", "").len(), MAX_MATCHES);
}

#[test]
fn trailing_space_hands_the_previous_words_to_the_matcher() {
    // UI 交给匹配层的只有「正在输入的词」和它之前的文本：`docker p` 里词是 `p`，
    // 前文 `docker ` 决定这是 docker 的参数位。
    let line = "docker p";
    let token = last_token(line);
    assert_eq!(token, "p");
    assert_eq!(
        token_slot(&line[..line.len() - token.len()]),
        TokenSlot::Argument
    );
    // 行尾只有一个空格时词为空，前文仍是整段命令；分隔符之后重新算命令位。
    assert_eq!(last_token("sudo apt "), "");
    assert_eq!(token_slot("sudo apt "), TokenSlot::Argument);
    assert_eq!(token_slot("cd /tmp && "), TokenSlot::Command);
}

#[test]
fn last_token_splits_on_separators() {
    assert_eq!(last_token("sudo apt upd"), "upd");
    assert_eq!(last_token("cd /tmp && ls"), "ls");
    assert_eq!(last_token(""), "");
}

#[test]
fn merged_command_skips_duplicate_leading_words() {
    assert_eq!(merged_command("sudo ", "sudo apt update"), "apt update");
    assert_eq!(
        merged_command("sudo apt ", "sudo apt install <包名>"),
        "install <包名>"
    );
    assert_eq!(merged_command("docker ", "docker ps -a"), "ps -a");
    assert_eq!(merged_command("cd /tmp && ", "ls -la"), "ls -la");
    assert_eq!(merged_command("", "df -h"), "df -h");
}

fn chars(text: &str) -> Vec<char> {
    text.chars().collect()
}

#[test]
fn echo_lagging_accepts_only_partial_echo_of_the_tail() {
    // 词已完整回显：后面只差一个空格的回显，替换仍然安全。
    assert!(echo_lagging(&chars("root# docker "), &chars("docker p"), 1));
    // 网格停在词前：连词都没回显，只是看起来像，不能动。
    assert!(!echo_lagging(&chars("root# docker"), &chars("docker p"), 1));
    // 正在输入的词只回显了前半截。
    assert!(echo_lagging(
        &chars("root# sudo dock"),
        &chars("sudo docker"),
        6
    ));
    // 整行都没回显。
    assert!(!echo_lagging(&chars("root# "), &chars("sudo docker"), 6));
    // 命令词在行首、且一个字符都没回显：没有锚点。
    assert!(!echo_lagging(&chars("root#"), &chars("docker"), 6));
    // 网格内容与记录的行完全无关。
    assert!(!echo_lagging(&chars("root# sudo"), &chars("echo hi"), 5));
    assert!(!echo_lagging(&[], &chars("docker"), 6));
}

#[test]
fn pool_builds_history_first_and_dedupes_against_snippets() {
    let snippets = vec![super::Snippet {
        id: crate::ids::SnippetId::new(),
        name: "磁盘余量".to_string(),
        command: "df -h".to_string(),
    }];
    let history = vec![
        "df -h /".to_string(),
        "df -h".to_string(),
        "git status".to_string(),
    ];
    let pool = CompletionPool::build(&history, &snippets);
    let items = pool.items();
    // 历史在最前（与片段同文的 df -h 被去重），片段在后。
    assert_eq!(items[0].kind, CompletionKind::History);
    assert_eq!(items[0].command, "df -h /");
    assert_eq!(items[1].kind, CompletionKind::History);
    assert_eq!(items[1].command, "git status");
    assert_eq!(items[2].kind, CompletionKind::Snippet);
    assert_eq!(items[2].command, "df -h");
    // 命令位模糊搜索同时命中两边的来源：历史条目按"最近使用"排最前。
    let hits = pool.complete("df", "");
    assert_eq!(hits[0].kind, CompletionKind::History);
    assert_eq!(hits[0].command, "df -h /");
    assert_eq!(hits[1].kind, CompletionKind::Snippet);
    assert_eq!(hits[1].command, "df -h");
    // 敲「git」只能命中历史那条。
    let hits = pool.complete("git", "");
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].command, "git status");
}

#[test]
fn pool_fingerprint_follows_content_and_order() {
    let snippets: Vec<super::Snippet> = (0..2)
        .map(|_| super::Snippet {
            id: crate::ids::SnippetId::new(),
            name: String::new(),
            command: String::new(),
        })
        .collect();
    let empty = CompletionPool::fingerprint(&[], &[]);
    let one = CompletionPool::fingerprint(&[String::from("ls")], &[]);
    let two = CompletionPool::fingerprint(&[String::from("ls"), String::from("pwd")], &[]);
    assert_ne!(empty, one);
    assert_ne!(one, two);
    assert_ne!(
        two,
        CompletionPool::fingerprint(&[String::from("pwd"), String::from("ls")], &[])
    );
    // 片段内容变化也会改变指纹（名称与命令参与哈希，id 不参与）。
    let mut renamed = snippets.clone();
    renamed[0].command = "df -h".to_string();
    assert_ne!(
        CompletionPool::fingerprint(&[], &snippets),
        CompletionPool::fingerprint(&[], &renamed)
    );
}
