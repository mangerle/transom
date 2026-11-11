//! 全局文件类型图标解析。
//!
//! 设计原理（Why）：文件图标必须由路径稳定推导，不能在各个列表里按“修改状态”
//! 或“是否脏”临时选择。统一解析器让 Git、SFTP、编辑器和容器文件列表共享同一套
//! 扩展名与完整文件名规则；资产使用 JetBrains IntelliJ Platform 的 Apache 2.0
//! 文件类型图标，并优先保留明暗两套 SVG。
//!
//! 代价与局限：新增语言类型时需要同时补充 SVG 资产与映射测试；未知扩展名回落到
//! 通用文本图标，避免不同模块各自发明默认图标。

use std::borrow::Cow;
use std::path::Path;

use gpui_kit::SharedString;
use rust_embed::RustEmbed;

/// 文件类型图标在资产源中的目录。
pub const FILE_TYPE_ICON_DIR: &str = "icons/file-types";

#[derive(RustEmbed)]
#[folder = "assets/icons/file-types"]
struct FileTypeAssets;

/// 一个文件类型图标；内部 stem 对应 `assets/icons/file-types/<stem>.svg`。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FileTypeIcon(&'static str);

impl FileTypeIcon {
    pub const ARCHIVE: Self = Self("archive");
    pub const C: Self = Self("c");
    pub const CPP: Self = Self("cpp");
    pub const CSHARP: Self = Self("Csharp");
    pub const ACTION_SCRIPT: Self = Self("actionScript");
    pub const ASPECT_J: Self = Self("aspectJ");
    pub const CLOJURE: Self = Self("clojure");
    pub const CONFIG: Self = Self("config");
    pub const CSS: Self = Self("css");
    pub const CSV: Self = Self("csv");
    pub const DART: Self = Self("dart");
    pub const DOCKER: Self = Self("docker");
    pub const EDITOR_CONFIG: Self = Self("editorConfig");
    pub const ELIXIR: Self = Self("elixir");
    pub const ERLANG: Self = Self("erlang");
    pub const GITIGNORE: Self = Self("gitignore");
    pub const GO: Self = Self("go");
    pub const GRADLE: Self = Self("gradle");
    pub const GRAPHQL: Self = Self("graphql");
    pub const GROOVY: Self = Self("groovy");
    pub const HASKELL: Self = Self("haskell");
    pub const HEADER: Self = Self("h");
    pub const HTML: Self = Self("html");
    pub const IMAGE: Self = Self("image");
    pub const JAVA: Self = Self("java");
    pub const JAVA_CLASS: Self = Self("javaClass");
    pub const JAVASCRIPT: Self = Self("javaScript");
    pub const JSON: Self = Self("json");
    pub const JSON_SCHEMA: Self = Self("jsonSchema");
    pub const JULIA: Self = Self("julia");
    pub const KOTLIN: Self = Self("kotlin");
    pub const LUA: Self = Self("lua");
    pub const MARKDOWN: Self = Self("markdown");
    pub const OBJECTIVE_C: Self = Self("objective-c");
    pub const OBJECTIVE_CPP: Self = Self("objective-cpp");
    pub const PATCH: Self = Self("patch");
    pub const PERL: Self = Self("perl");
    pub const PHP: Self = Self("php");
    pub const PROPERTIES: Self = Self("properties");
    pub const PYTHON: Self = Self("python");
    pub const R: Self = Self("r");
    pub const REGEXP: Self = Self("regexp");
    pub const RST: Self = Self("rst");
    pub const RUBY: Self = Self("ruby");
    pub const RUST: Self = Self("rust");
    pub const SCALA: Self = Self("scala");
    pub const SHELL: Self = Self("shell");
    pub const SQL: Self = Self("sql");
    pub const SVELTE: Self = Self("svelte");
    pub const SWIFT: Self = Self("swift");
    pub const TEXT: Self = Self("text");
    pub const TOML: Self = Self("toml");
    pub const TYPESCRIPT: Self = Self("typeScript");
    pub const UNKNOWN: Self = Self("unknown");
    pub const VUE: Self = Self("vue");
    pub const XML: Self = Self("xml");
    pub const XSD: Self = Self("xsd");
    pub const YAML: Self = Self("yaml");
    pub const ZIG: Self = Self("zig");

    /// 根据文件名、完整文件名和扩展名解析图标。
    ///
    /// 优先级固定为完整文件名、双后缀、普通扩展名，确保 `Cargo.toml`、
    /// `types.d.ts` 这类文件不会被普通后缀规则抢占。
    pub fn for_path(path: &str) -> Self {
        let file_name = Path::new(path)
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or(path)
            .to_ascii_lowercase();

        let exact = match file_name.as_str() {
            "dockerfile" | "containerfile" => Some(Self::DOCKER),
            ".gitignore" | ".gitattributes" | ".gitmodules" | ".dockerignore" => {
                Some(Self::GITIGNORE)
            }
            ".editorconfig" => Some(Self::EDITOR_CONFIG),
            "cargo.toml" | "cargo.lock" => Some(Self::TOML),
            "pom.xml" | "build.xml" | "web.xml" => Some(Self::XML),
            "readme" | "license" | "copying" | "authors" | "notice" => Some(Self::TEXT),
            "gemfile" | "rakefile" | "podfile" => Some(Self::RUBY),
            _ => None,
        };
        if let Some(icon) = exact {
            return icon;
        }

        if file_name.ends_with(".d.ts") || file_name.ends_with(".d.mts") {
            return Self::TYPESCRIPT;
        }
        if [".tar.gz", ".tar.bz2", ".tar.xz", ".tar.zst"]
            .iter()
            .any(|suffix| file_name.ends_with(suffix))
        {
            return Self::ARCHIVE;
        }

        let extension = file_name.rsplit_once('.').map_or("", |(_, ext)| ext);
        match extension {
            "rs" => Self::RUST,
            "java" => Self::JAVA,
            "class" => Self::JAVA_CLASS,
            "kt" | "kts" => Self::KOTLIN,
            "py" | "pyi" | "pyw" => Self::PYTHON,
            "go" => Self::GO,
            "php" | "phtml" => Self::PHP,
            "rb" | "ru" => Self::RUBY,
            "scala" | "sc" => Self::SCALA,
            "dart" => Self::DART,
            "swift" => Self::SWIFT,
            "lua" => Self::LUA,
            "hs" | "lhs" => Self::HASKELL,
            "ex" | "exs" => Self::ELIXIR,
            "erl" | "hrl" => Self::ERLANG,
            "clj" | "cljs" | "cljc" | "edn" => Self::CLOJURE,
            "r" => Self::R,
            "jl" => Self::JULIA,
            "zig" => Self::ZIG,
            "m" => Self::OBJECTIVE_C,
            "mm" => Self::OBJECTIVE_CPP,
            "vue" => Self::VUE,
            "svelte" => Self::SVELTE,
            "cs" => Self::CSHARP,
            "c" => Self::C,
            "h" => Self::HEADER,
            "cc" | "cpp" | "cxx" | "hh" | "hpp" | "hxx" => Self::CPP,
            "js" | "mjs" | "cjs" | "jsx" => Self::JAVASCRIPT,
            "ts" | "tsx" | "mts" | "cts" => Self::TYPESCRIPT,
            "as" => Self::ACTION_SCRIPT,
            "aj" => Self::ASPECT_J,
            "graphql" | "gql" => Self::GRAPHQL,
            "groovy" => Self::GROOVY,
            "gradle" => Self::GRADLE,
            "css" | "scss" | "sass" | "less" => Self::CSS,
            "csv" => Self::CSV,
            "htm" | "html" | "xhtml" => Self::HTML,
            "json" | "jsonc" => Self::JSON,
            "json5" => Self::JSON_SCHEMA,
            "md" | "markdown" | "mdx" => Self::MARKDOWN,
            "patch" | "diff" => Self::PATCH,
            "pl" | "pm" => Self::PERL,
            "properties" => Self::PROPERTIES,
            "re" | "regexp" => Self::REGEXP,
            "rst" => Self::RST,
            "sh" | "bash" | "zsh" | "fish" => Self::SHELL,
            "sql" => Self::SQL,
            "toml" => Self::TOML,
            "xml" | "plist" | "jsp" | "jspx" => Self::XML,
            "xsd" | "wsdl" => Self::XSD,
            "yaml" | "yml" => Self::YAML,
            "ini" | "cfg" | "conf" | "env" | "service" => Self::CONFIG,
            "txt" | "log" => Self::TEXT,
            "png" | "jpg" | "jpeg" | "gif" | "bmp" | "webp" | "svg" | "ico" => Self::IMAGE,
            "zip" | "tar" | "gz" | "tgz" | "bz2" | "xz" | "zst" | "7z" | "rar" | "jar" => {
                Self::ARCHIVE
            }
            "" => Self::UNKNOWN,
            _ => Self::TEXT,
        }
    }

    /// 当前图标的浅色资产路径。
    pub fn light_path(self) -> SharedString {
        format!("{FILE_TYPE_ICON_DIR}/{}.svg", self.0).into()
    }

    #[cfg(test)]
    fn stem(self) -> &'static str {
        self.0
    }
}

/// 从嵌入资产读取指定文件类型图标。
pub fn asset_data(path: &str) -> Option<Cow<'static, [u8]>> {
    let relative = path.strip_prefix(FILE_TYPE_ICON_DIR)?.strip_prefix('/')?;
    FileTypeAssets::get(relative).map(|asset| asset.data)
}

/// 列出全部文件类型图标资产。
pub fn asset_paths() -> impl Iterator<Item = SharedString> {
    FileTypeAssets::iter()
        .filter(|name| name.ends_with(".svg"))
        .map(|name| format!("{FILE_TYPE_ICON_DIR}/{name}").into())
}

/// 若路径是文件类型图标的浅色版本且有深色资产，返回深色路径。
pub fn dark_variant(path: &str) -> Option<SharedString> {
    let relative = path.strip_prefix(FILE_TYPE_ICON_DIR)?.strip_prefix('/')?;
    let stem = relative.strip_suffix(".svg")?;
    if stem.ends_with("_dark") {
        return None;
    }
    let dark_file = format!("{stem}_dark.svg");
    FileTypeAssets::get(&dark_file).map(|_| format!("{FILE_TYPE_ICON_DIR}/{dark_file}").into())
}

#[cfg(test)]
mod tests {
    use super::{FileTypeIcon, asset_data, dark_variant};

    #[test]
    fn exact_names_override_extensions() {
        assert_eq!(FileTypeIcon::for_path("Cargo.toml").stem(), "toml");
        assert_eq!(FileTypeIcon::for_path("Cargo.lock").stem(), "toml");
        assert_eq!(FileTypeIcon::for_path("Dockerfile").stem(), "docker");
        assert_eq!(FileTypeIcon::for_path(".gitignore").stem(), "gitignore");
    }

    #[test]
    fn extensions_are_case_insensitive_and_support_double_suffixes() {
        assert_eq!(FileTypeIcon::for_path("SRC/MAIN.RS").stem(), "rust");
        assert_eq!(FileTypeIcon::for_path("types.d.ts").stem(), "typeScript");
        assert_eq!(FileTypeIcon::for_path("archive.tar.gz").stem(), "archive");
        assert_eq!(FileTypeIcon::for_path("scripts/deploy.py").stem(), "python");
        assert_eq!(FileTypeIcon::for_path("src/App.kt").stem(), "kotlin");
        assert_eq!(FileTypeIcon::for_path("cmd/server.go").stem(), "go");
    }

    #[test]
    fn unsupported_extensions_fall_back_to_text_and_no_extension_to_unknown() {
        assert_eq!(FileTypeIcon::for_path("script.unknown").stem(), "text");
        assert_eq!(FileTypeIcon::for_path("LICENSE_LOCAL").stem(), "unknown");
    }

    #[test]
    fn embedded_assets_and_dark_variants_exist() {
        assert!(asset_data("icons/file-types/rust.svg").is_some());
        assert!(asset_data("icons/file-types/rust_dark.svg").is_some());
        assert!(asset_data("icons/file-types/python.svg").is_some());
        assert!(asset_data("icons/file-types/python_dark.svg").is_some());
        assert_eq!(
            dark_variant("icons/file-types/java.svg").as_deref(),
            Some("icons/file-types/java_dark.svg")
        );
        assert!(dark_variant("icons/file-types/gitignore.svg").is_none());
    }
}
