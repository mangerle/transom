//! 请求认证编辑：按认证类型展示结构化字段。

use crate::theme::tokens::{space, typography};
use crate::widgets::dropdown;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::{ActiveTheme, Sizable};
use gpui_kit::*;
use transom_api::ApiAuth;

/// 认证类型下拉值。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AuthKind {
    None,
    Bearer,
    Basic,
    OAuth2ClientCredentials,
    AwsSigV4,
}

impl AuthKind {
    const ALL: [Self; 5] = [
        Self::None,
        Self::Bearer,
        Self::Basic,
        Self::OAuth2ClientCredentials,
        Self::AwsSigV4,
    ];

    const fn label(self) -> &'static str {
        match self {
            Self::None => "api/auth/none",
            Self::Bearer => "api/auth/bearer",
            Self::Basic => "api/auth/basic",
            Self::OAuth2ClientCredentials => "api/auth/oauth2",
            Self::AwsSigV4 => "api/auth/aws_sigv4",
        }
    }

    fn from_auth(auth: &ApiAuth) -> Self {
        match auth {
            ApiAuth::None => Self::None,
            ApiAuth::Bearer { .. } => Self::Bearer,
            ApiAuth::Basic { .. } => Self::Basic,
            ApiAuth::OAuth2ClientCredentials { .. } => Self::OAuth2ClientCredentials,
            ApiAuth::AwsSigV4 { .. } => Self::AwsSigV4,
        }
    }
}

struct AuthFields {
    bearer_token: Entity<InputState>,
    basic_username: Entity<InputState>,
    basic_password: Entity<InputState>,
    oauth_token_url: Entity<InputState>,
    oauth_client_id: Entity<InputState>,
    oauth_secret: Entity<InputState>,
    oauth_scope: Entity<InputState>,
    aws_access_key: Entity<InputState>,
    aws_secret_key: Entity<InputState>,
    aws_session_token: Entity<InputState>,
    aws_region: Entity<InputState>,
    aws_service: Entity<InputState>,
}

/// 请求认证表单变化事件。
pub(crate) enum AuthEditorEvent {
    Changed(ApiAuth),
}

/// 结构化认证编辑器。
pub(crate) struct AuthEditor {
    kind: AuthKind,
    fields: AuthFields,
    syncing: bool,
    subscriptions: Vec<Subscription>,
}

impl EventEmitter<AuthEditorEvent> for AuthEditor {}

impl AuthEditor {
    pub(crate) fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut field = |cx: &mut Context<Self>, key: &'static str| {
            let placeholder = rust_i18n::t!(key).to_string();
            cx.new(|cx| InputState::new(window, cx).placeholder(placeholder))
        };
        let fields = AuthFields {
            bearer_token: field(cx, "api.request.auth_method.token_env_placeholder"),
            basic_username: field(cx, "api.request.auth_method.username_env_placeholder"),
            basic_password: field(cx, "api.request.auth_method.password_env_placeholder"),
            oauth_token_url: field(cx, "api.request.auth_method.token_url_placeholder"),
            oauth_client_id: field(cx, "api.request.auth_method.client_id_placeholder"),
            oauth_secret: field(cx, "api.request.auth_method.client_secret_env_placeholder"),
            oauth_scope: field(cx, "api.request.auth_method.scope_placeholder"),
            aws_access_key: field(cx, "api.request.auth_method.access_key_env_placeholder"),
            aws_secret_key: field(cx, "api.request.auth_method.secret_key_env_placeholder"),
            aws_session_token: field(cx, "api.request.auth_method.session_token_env_placeholder"),
            aws_region: field(cx, "api.request.auth_method.region_placeholder"),
            aws_service: field(cx, "api.request.auth_method.service_placeholder"),
        };
        let mut editor = Self {
            kind: AuthKind::None,
            fields,
            syncing: false,
            subscriptions: Vec::with_capacity(12),
        };
        let inputs = editor
            .inputs()
            .into_iter()
            .cloned()
            .collect::<Vec<Entity<InputState>>>();
        for input in inputs {
            let subscription =
                cx.subscribe_in(&input, window, |this, _, event: &InputEvent, _, cx| {
                    if matches!(event, InputEvent::Change) && !this.syncing {
                        this.report(cx);
                    }
                });
            editor.subscriptions.push(subscription);
        }
        editor
    }

    /// 从请求模型载入认证配置。
    pub(crate) fn set_auth(&mut self, auth: &ApiAuth, window: &mut Window, cx: &mut Context<Self>) {
        self.syncing = true;
        self.kind = AuthKind::from_auth(auth);
        let (
            bearer_token,
            basic_username,
            basic_password,
            oauth_token_url,
            oauth_client_id,
            oauth_secret,
            oauth_scope,
            aws_access_key,
            aws_secret_key,
            aws_session_token,
            aws_region,
            aws_service,
        ) = match auth {
            ApiAuth::None => Default::default(),
            ApiAuth::Bearer { token_env } => (
                token_env.clone(),
                String::new(),
                String::new(),
                String::new(),
                String::new(),
                String::new(),
                String::new(),
                String::new(),
                String::new(),
                String::new(),
                String::new(),
                String::new(),
            ),
            ApiAuth::Basic {
                username_env,
                password_env,
            } => (
                String::new(),
                username_env.clone(),
                password_env.clone(),
                String::new(),
                String::new(),
                String::new(),
                String::new(),
                String::new(),
                String::new(),
                String::new(),
                String::new(),
                String::new(),
            ),
            ApiAuth::OAuth2ClientCredentials {
                token_url,
                client_id,
                client_secret_env,
                scope,
            } => (
                String::new(),
                String::new(),
                String::new(),
                token_url.clone(),
                client_id.clone(),
                client_secret_env.clone(),
                scope.clone(),
                String::new(),
                String::new(),
                String::new(),
                String::new(),
                String::new(),
            ),
            ApiAuth::AwsSigV4 {
                access_key_env,
                secret_key_env,
                session_token_env,
                region,
                service,
            } => (
                String::new(),
                String::new(),
                String::new(),
                String::new(),
                String::new(),
                String::new(),
                String::new(),
                access_key_env.clone(),
                secret_key_env.clone(),
                session_token_env.clone(),
                region.clone(),
                service.clone(),
            ),
        };
        set_input(&self.fields.bearer_token, bearer_token, window, cx);
        set_input(&self.fields.basic_username, basic_username, window, cx);
        set_input(&self.fields.basic_password, basic_password, window, cx);
        set_input(&self.fields.oauth_token_url, oauth_token_url, window, cx);
        set_input(&self.fields.oauth_client_id, oauth_client_id, window, cx);
        set_input(&self.fields.oauth_secret, oauth_secret, window, cx);
        set_input(&self.fields.oauth_scope, oauth_scope, window, cx);
        set_input(&self.fields.aws_access_key, aws_access_key, window, cx);
        set_input(&self.fields.aws_secret_key, aws_secret_key, window, cx);
        set_input(
            &self.fields.aws_session_token,
            aws_session_token,
            window,
            cx,
        );
        set_input(&self.fields.aws_region, aws_region, window, cx);
        set_input(&self.fields.aws_service, aws_service, window, cx);
        self.syncing = false;
        cx.notify();
    }

    /// 读取当前认证配置；空字段仍保留，发送时由认证服务给出明确错误。
    pub(crate) fn auth(&self, cx: &App) -> ApiAuth {
        let read = |input: &Entity<InputState>| input.read(cx).value().trim().to_string();
        match self.kind {
            AuthKind::None => ApiAuth::None,
            AuthKind::Bearer => ApiAuth::Bearer {
                token_env: read(&self.fields.bearer_token),
            },
            AuthKind::Basic => ApiAuth::Basic {
                username_env: read(&self.fields.basic_username),
                password_env: read(&self.fields.basic_password),
            },
            AuthKind::OAuth2ClientCredentials => ApiAuth::OAuth2ClientCredentials {
                token_url: read(&self.fields.oauth_token_url),
                client_id: read(&self.fields.oauth_client_id),
                client_secret_env: read(&self.fields.oauth_secret),
                scope: read(&self.fields.oauth_scope),
            },
            AuthKind::AwsSigV4 => ApiAuth::AwsSigV4 {
                access_key_env: read(&self.fields.aws_access_key),
                secret_key_env: read(&self.fields.aws_secret_key),
                session_token_env: read(&self.fields.aws_session_token),
                region: read(&self.fields.aws_region),
                service: read(&self.fields.aws_service),
            },
        }
    }

    fn inputs(&self) -> [&Entity<InputState>; 12] {
        [
            &self.fields.bearer_token,
            &self.fields.basic_username,
            &self.fields.basic_password,
            &self.fields.oauth_token_url,
            &self.fields.oauth_client_id,
            &self.fields.oauth_secret,
            &self.fields.oauth_scope,
            &self.fields.aws_access_key,
            &self.fields.aws_secret_key,
            &self.fields.aws_session_token,
            &self.fields.aws_region,
            &self.fields.aws_service,
        ]
    }

    fn set_kind(&mut self, kind: AuthKind, cx: &mut Context<Self>) {
        self.kind = kind;
        self.report(cx);
    }

    fn report(&mut self, cx: &mut Context<Self>) {
        cx.emit(AuthEditorEvent::Changed(self.auth(cx)));
        cx.notify();
    }
}

impl Render for AuthEditor {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let view = cx.entity();
        let kind = dropdown(
            "api-auth-kind",
            self.kind,
            AuthKind::ALL
                .into_iter()
                .map(|kind| (kind, rust_i18n::t!(kind.label()).to_string()))
                .collect(),
            |this: &mut Self, kind, _, cx| this.set_kind(kind, cx),
            view,
        );
        let mut body = v_flex().w_full().gap(rems(space::SM));
        match self.kind {
            AuthKind::None => {
                body = body.child(
                    div()
                        .text_color(theme.muted_foreground)
                        .child(rust_i18n::t!("api.request.auth_method.none_hint").to_string()),
                );
            }
            AuthKind::Bearer => {
                body = body.child(field_row(
                    rust_i18n::t!("api.request.auth_method.token_env").to_string(),
                    &self.fields.bearer_token,
                    theme.muted_foreground,
                ));
            }
            AuthKind::Basic => {
                body = body
                    .child(field_row(
                        rust_i18n::t!("api.request.auth_method.username_env").to_string(),
                        &self.fields.basic_username,
                        theme.muted_foreground,
                    ))
                    .child(field_row(
                        rust_i18n::t!("api.request.auth_method.password_env").to_string(),
                        &self.fields.basic_password,
                        theme.muted_foreground,
                    ));
            }
            AuthKind::OAuth2ClientCredentials => {
                body = body
                    .child(field_row(
                        rust_i18n::t!("api.request.auth_method.token_url").to_string(),
                        &self.fields.oauth_token_url,
                        theme.muted_foreground,
                    ))
                    .child(field_row(
                        rust_i18n::t!("api.request.auth_method.client_id").to_string(),
                        &self.fields.oauth_client_id,
                        theme.muted_foreground,
                    ))
                    .child(field_row(
                        rust_i18n::t!("api.request.auth_method.client_secret_env").to_string(),
                        &self.fields.oauth_secret,
                        theme.muted_foreground,
                    ))
                    .child(field_row(
                        rust_i18n::t!("api.request.auth_method.scope").to_string(),
                        &self.fields.oauth_scope,
                        theme.muted_foreground,
                    ));
            }
            AuthKind::AwsSigV4 => {
                body = body
                    .child(field_row(
                        rust_i18n::t!("api.request.auth_method.access_key_env").to_string(),
                        &self.fields.aws_access_key,
                        theme.muted_foreground,
                    ))
                    .child(field_row(
                        rust_i18n::t!("api.request.auth_method.secret_key_env").to_string(),
                        &self.fields.aws_secret_key,
                        theme.muted_foreground,
                    ))
                    .child(field_row(
                        rust_i18n::t!("api.request.auth_method.session_token_env").to_string(),
                        &self.fields.aws_session_token,
                        theme.muted_foreground,
                    ))
                    .child(field_row(
                        rust_i18n::t!("api.request.auth_method.region").to_string(),
                        &self.fields.aws_region,
                        theme.muted_foreground,
                    ))
                    .child(field_row(
                        rust_i18n::t!("api.request.auth_method.service").to_string(),
                        &self.fields.aws_service,
                        theme.muted_foreground,
                    ));
            }
        }
        v_flex()
            .id("api-auth-editor")
            .size_full()
            .min_h_0()
            .p(rems(space::SM))
            .gap(rems(space::SM))
            .child(kind)
            .child(div().flex_1().min_h_0().overflow_y_scrollbar().child(body))
            .into_any_element()
    }
}

fn field_row(label: String, input: &Entity<InputState>, color: Hsla) -> AnyElement {
    h_flex()
        .w_full()
        .gap(rems(space::SM))
        .items_center()
        .child(
            div()
                .w(rems(11.0))
                .flex_none()
                .text_color(color)
                .text_size(rems(typography::CAPTION))
                .child(label),
        )
        .child(Input::new(input).flex_1().min_w_0().small())
        .into_any_element()
}

fn set_input(
    input: &Entity<InputState>,
    value: String,
    window: &mut Window,
    cx: &mut Context<AuthEditor>,
) {
    input.update(cx, |input, cx| input.set_value(value, window, cx));
}
