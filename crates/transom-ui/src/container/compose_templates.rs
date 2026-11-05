//! Docker Compose 常用中间件与服务应用模板库。
//!
//! ### 设计初衷 (Why)
//! 在使用 Compose 管理微服务时，手动编写 YAML 存在大量格式与参数心智负担。
//! 提供经过真机验证的常用中间件（Nginx, Redis, PostgreSQL, MySQL, 全栈微服务栈）模板库，
//! 开发者可一键生成标准模板并直接在线编辑运行，大幅降低容器化试错成本。
//!
//! ### 核心优势
//! 内置数据卷持久化（Volume）声明与端口映射，开箱即用；模板经强类型 YAML 解析验证，杜绝缩进错误。
//!
//! ### 代价与局限
//! 模板面向通用开发与本地调试环境，生产高可用部署仍需按需调优安全密码与持久化路径。

/// Compose 预设模板契约。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ComposeTemplate {
    pub(crate) id: &'static str,
    pub(crate) name: &'static str,
    pub(crate) desc: &'static str,
    pub(crate) yaml: &'static str,
}

pub(crate) const COMPOSE_TEMPLATES: &[ComposeTemplate] = &[
    ComposeTemplate {
        id: "minimal",
        name: "极简服务 (Alpine)",
        desc: "最轻量常驻容器，适合快速测试连通性与执行调试命令",
        yaml: r#"services:
  app:
    image: alpine:3.20
    command: ["sh", "-c", "while true; do sleep 3600; done"]
    restart: unless-stopped
"#,
    },
    ComposeTemplate {
        id: "nginx",
        name: "Nginx 反向代理与 Web 服务",
        desc: "高性能 HTTP 静态托管与反向代理，映射 80 端口并持久化静态目录",
        yaml: r#"services:
  web:
    image: nginx:alpine
    ports:
      - "80:80"
    volumes:
      - nginx-html:/usr/share/nginx/html:ro
    restart: unless-stopped

volumes:
  nginx-html:
"#,
    },
    ComposeTemplate {
        id: "redis",
        name: "Redis 键值缓存服务",
        desc: "内存数据库与分布式缓存，开启 AOF 持久化并映射 6379 端口",
        yaml: r#"services:
  redis:
    image: redis:7-alpine
    command: ["redis-server", "--appendonly", "yes"]
    ports:
      - "6379:6379"
    volumes:
      - redis-data:/data
    restart: unless-stopped

volumes:
  redis-data:
"#,
    },
    ComposeTemplate {
        id: "postgres",
        name: "PostgreSQL 关系型数据库",
        desc: "企业级 SQL 数据库，默认配置持久化卷并映射 5432 端口",
        yaml: r#"services:
  postgres:
    image: postgres:16-alpine
    environment:
      POSTGRES_DB: app_db
      POSTGRES_USER: postgres
      POSTGRES_PASSWORD: changeme
    ports:
      - "5432:5432"
    volumes:
      - postgres-data:/var/lib/postgresql/data
    restart: unless-stopped

volumes:
  postgres-data:
"#,
    },
    ComposeTemplate {
        id: "mysql",
        name: "MySQL 8.0 关系型数据库",
        desc: "流行开源关系型数据库，映射 3306 端口并挂载持久化存储",
        yaml: r#"services:
  mysql:
    image: mysql:8.0
    environment:
      MYSQL_ROOT_PASSWORD: rootpassword
      MYSQL_DATABASE: my_database
      MYSQL_USER: dbuser
      MYSQL_PASSWORD: dbpassword
    ports:
      - "3306:3306"
    volumes:
      - mysql-data:/var/lib/mysql
    restart: unless-stopped

volumes:
  mysql-data:
"#,
    },
    ComposeTemplate {
        id: "fullstack",
        name: "全栈服务架构 (Nginx + Redis + PostgreSQL)",
        desc: "多服务联动微服务拓扑：前端网关、内存缓存与数据库依赖编排",
        yaml: r#"services:
  web:
    image: nginx:alpine
    ports:
      - "8080:80"
    depends_on:
      - redis
      - db
    restart: unless-stopped

  redis:
    image: redis:7-alpine
    restart: unless-stopped

  db:
    image: postgres:16-alpine
    environment:
      POSTGRES_DB: fullstack_db
      POSTGRES_PASSWORD: secretpassword
    volumes:
      - db-data:/var/lib/postgresql/data
    restart: unless-stopped

volumes:
  db-data:
"#,
    },
];

/// 获取所有内置的 Compose 模板。
pub(crate) fn default_templates() -> &'static [ComposeTemplate] {
    COMPOSE_TEMPLATES
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_all_compose_templates_are_valid_yaml() {
        for t in COMPOSE_TEMPLATES {
            let res: Result<serde_json::Value, _> = serde_yaml_ng::from_str(t.yaml);
            assert!(
                res.is_ok(),
                "Compose 模板 '{}' 不是合法的 YAML 格式: {:?}",
                t.name,
                res.err()
            );
        }
    }
}
