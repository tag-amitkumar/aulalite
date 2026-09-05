use crate::contexts::{UserContext, UserContextSignal};
use dioxus::prelude::*;
use features_courses::api::{self, ApiContext};
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BootstrapDecision {
    SignedIn,
    Anonymous,
    StaleToken,
}

pub fn decide_bootstrap_outcome(had_token: bool, me_ok: bool) -> BootstrapDecision {
    match (had_token, me_ok) {
        (true, true) => BootstrapDecision::SignedIn,
        (true, false) => BootstrapDecision::StaleToken,
        (false, _) => BootstrapDecision::Anonymous,
    }
}

pub struct AuthBootstrapState {
    pub api_ctx_signal: Signal<ApiContext>,
    pub user_ctx_signal: UserContextSignal,
    pub auth_ready: Signal<bool>,
}

#[component]
pub fn AuthSplash() -> Element {
    rsx! {
        div { class: "app-auth-splash", "aria-busy": "true",
            span { class: "app-auth-splash-mark", "Aula", span { class: "gold", "Lite" } }
        }
    }
}

pub fn use_auth_bootstrap() -> AuthBootstrapState {
    #[cfg(target_arch = "wasm32")]
    let initial_base_url = api::web_api_base_url();
    #[cfg(not(target_arch = "wasm32"))]
    let initial_base_url = api::native_api_base_url();

    let mut api_ctx_signal = use_signal(|| ApiContext {
        base_url: initial_base_url,
        id_token: String::new(),
    });
    let mut user_ctx_signal: UserContextSignal = use_signal(|| None);
    let mut bootstrapped = use_signal(|| false);
    let mut auth_ready = use_signal(|| false);

    use_hook(|| {
        #[cfg(target_arch = "wasm32")]
        {
            use platform_bridge::PlatformBridge;
            let refresh: api::RefreshFn = Arc::new(|| {
                Box::pin(async move {
                    let bridge = platform_bridge::web::WebBridge;
                    bridge.current_id_token().await.map_err(|e| format!("{e}"))
                })
            });
            api::set_refresh_fn(refresh);
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            use platform_bridge::PlatformBridge;
            let refresh: api::RefreshFn = Arc::new(|| {
                Box::pin(async move {
                    let bridge = platform_bridge::native::NativeBridge;
                    bridge.current_id_token().await.map_err(|e| format!("{e}"))
                })
            });
            api::set_refresh_fn(refresh);
        }
    });

    use_future(move || async move {
        if *bootstrapped.read() {
            return;
        }
        bootstrapped.set(true);

        #[cfg(target_arch = "wasm32")]
        {
            use platform_bridge::PlatformBridge;
            let bridge = platform_bridge::web::WebBridge;

            if let Some(token) = platform_bridge::web::local_token() {
                let bootstrap_ctx = ApiContext {
                    base_url: api::web_api_base_url(),
                    id_token: token.clone(),
                };
                match api::get_me(&bootstrap_ctx).await {
                    Ok(dto) => {
                        api_ctx_signal.set(bootstrap_ctx);
                        user_ctx_signal.set(Some(UserContext::from_dto(dto)));
                        auth_ready.set(true);
                        return;
                    }
                    Err(_) => {
                        platform_bridge::web::clear_local_token();
                    }
                }
            }

            match bridge.current_id_token().await {
                Ok(token) => {
                    if !api_ctx_signal.read().id_token.is_empty() {
                        auth_ready.set(true);
                        return;
                    }
                    let bootstrap_ctx = ApiContext {
                        base_url: api::web_api_base_url(),
                        id_token: token.clone(),
                    };
                    api_ctx_signal.set(bootstrap_ctx.clone());
                    match api::get_me(&bootstrap_ctx).await {
                        Ok(dto) => {
                            if api_ctx_signal.read().id_token == token {
                                user_ctx_signal.set(Some(UserContext::from_dto(dto)));
                            }
                        }
                        Err(_) => {
                            if api_ctx_signal.read().id_token == token {
                                user_ctx_signal.set(None);
                            }
                        }
                    }
                }
                Err(_) => {
                    if api_ctx_signal.read().id_token.is_empty() {
                        user_ctx_signal.set(None);
                    }
                }
            }
            auth_ready.set(true);
        }

        #[cfg(not(target_arch = "wasm32"))]
        {
            use platform_bridge::PlatformBridge;
            let bridge = platform_bridge::native::NativeBridge;
            match bridge.current_id_token().await {
                Ok(token) => {
                    if !api_ctx_signal.read().id_token.is_empty() {
                        auth_ready.set(true);
                        return;
                    }
                    let bootstrap_ctx = ApiContext {
                        base_url: api::native_api_base_url(),
                        id_token: token.clone(),
                    };
                    api_ctx_signal.set(bootstrap_ctx.clone());
                    match api::get_me(&bootstrap_ctx).await {
                        Ok(dto) => {
                            if api_ctx_signal.read().id_token == token {
                                user_ctx_signal.set(Some(UserContext::from_dto(dto)));
                            }
                        }
                        Err(_) => {
                            if api_ctx_signal.read().id_token == token {
                                user_ctx_signal.set(None);
                            }
                        }
                    }
                }
                Err(_) => {
                    if api_ctx_signal.read().id_token.is_empty() {
                        user_ctx_signal.set(None);
                    }
                }
            }
            auth_ready.set(true);
        }
    });

    use_context_provider::<Signal<ApiContext>>(|| api_ctx_signal);
    use_context_provider::<UserContextSignal>(|| user_ctx_signal);

    AuthBootstrapState {
        api_ctx_signal,
        user_ctx_signal,
        auth_ready,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bootstrap_decision_distinguishes_signed_in_stale_and_anonymous() {
        assert_eq!(
            decide_bootstrap_outcome(true, true),
            BootstrapDecision::SignedIn
        );
        assert_eq!(
            decide_bootstrap_outcome(true, false),
            BootstrapDecision::StaleToken
        );
        assert_eq!(
            decide_bootstrap_outcome(false, true),
            BootstrapDecision::Anonymous
        );
        assert_eq!(
            decide_bootstrap_outcome(false, false),
            BootstrapDecision::Anonymous
        );
    }

    #[test]
    fn auth_splash_renders_stable_busy_markup() {
        let mut vdom = VirtualDom::new(AuthSplash);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(html.contains("app-auth-splash"), "html: {html}");
        assert!(html.contains("aria-busy=\"true\""), "html: {html}");
        assert!(html.contains("Aula"), "html: {html}");
    }

    #[test]
    fn auth_bootstrap_hook_is_available_to_shells() {
        let hook: fn() -> AuthBootstrapState = super::use_auth_bootstrap;
        let _ = hook;
    }
}
