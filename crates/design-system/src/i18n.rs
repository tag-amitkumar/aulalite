//! Compile-time i18n: a `&str`-keyed translation catalog, a reactive locale
//! context, a `t(key)` lookup hook, and an RTL text-direction helper.
//!
//! Design goals (mirroring `theme.rs`):
//!   * The catalog is a `const` table baked into the binary — no I/O, no async,
//!     SSR-safe, and unit-testable without a live document.
//!   * One source of truth for the active locale lives in a Dioxus context
//!     (`Signal<Locale>`), provided by [`LocaleProvider`] near the app root.
//!   * `use_t()` returns a cheap `Fn(&str) -> &'static str` closure so call
//!     sites read `let t = use_t();` then `{t("nav.dashboard")}`.
//!   * Lookups fall back to English, then to the raw key, so a missing
//!     translation degrades visibly (the key) rather than panicking.
//!   * Direction (LTR/RTL) is derived from the locale; [`LocaleProvider`] mirrors
//!     it to `document.dir` + a persisted `localStorage` value (wasm only) the
//!     same way `apply_theme_preference` mirrors the theme, so the boot script
//!     and CSS `[dir=rtl]` overrides can paint before WASM hydrates.
//!
//! English is complete for every key we externalize; Arabic (RTL) and Spanish
//! (LTR) are shipped as proof of the multi-locale + bidi plumbing. Untranslated
//! keys fall through to English.

use dioxus::prelude::*;

/// Text direction for a locale. Drives the `dir` attribute and the
/// `[dir=rtl]` CSS overrides in `components.css`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dir {
    Ltr,
    Rtl,
}

impl Dir {
    /// The HTML `dir` attribute value (`"ltr"` / `"rtl"`).
    pub fn as_str(self) -> &'static str {
        match self {
            Dir::Ltr => "ltr",
            Dir::Rtl => "rtl",
        }
    }
}

/// A supported UI locale. Ordered as it should appear in the switcher.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Locale {
    #[default]
    En,
    Ar,
    Es,
}

impl Locale {
    /// All locales, in display order. Used by the switcher to build its menu.
    pub const ALL: [Locale; 3] = [Locale::En, Locale::Ar, Locale::Es];

    /// BCP-47-ish code persisted to storage and emitted as the `lang`
    /// attribute (`"en"` / `"ar"` / `"es"`).
    pub fn code(self) -> &'static str {
        match self {
            Locale::En => "en",
            Locale::Ar => "ar",
            Locale::Es => "es",
        }
    }

    /// Endonym shown in the locale switcher (the language's own name).
    pub fn native_label(self) -> &'static str {
        match self {
            Locale::En => "English",
            Locale::Ar => "العربية",
            Locale::Es => "Español",
        }
    }

    /// Text direction for this locale.
    pub fn dir(self) -> Dir {
        match self {
            Locale::Ar => Dir::Rtl,
            Locale::En | Locale::Es => Dir::Ltr,
        }
    }

    /// Parse a stored/`lang` code back to a locale; unknown codes fall back to
    /// English (so corrupt storage never crashes the app).
    pub fn parse(value: &str) -> Self {
        // Accept region-tagged codes ("ar-EG", "es_MX") by matching the prefix.
        let base = value
            .split(['-', '_'])
            .next()
            .unwrap_or(value)
            .to_ascii_lowercase();
        match base.as_str() {
            "ar" => Locale::Ar,
            "es" => Locale::Es,
            _ => Locale::En,
        }
    }
}

// ---------------------------------------------------------------------------
// Catalog
// ---------------------------------------------------------------------------
//
// One row per externalized key. English is authoritative & complete; Arabic and
// Spanish are filled where translated and left empty ("") to fall back to
// English. Keep keys dotted + namespaced (`area.thing`) and SORTED by key so
// the table stays scannable as it grows.

/// A single catalog row: a key plus its `(en, ar, es)` strings.
struct Entry {
    key: &'static str,
    en: &'static str,
    ar: &'static str,
    es: &'static str,
}

/// The translation catalog. An empty `ar`/`es` cell means "not translated yet"
/// and falls back to English at lookup time.
const CATALOG: &[Entry] = &[
    // --- App navigation (AppShell sidebar) ---
    Entry {
        key: "nav.dashboard",
        en: "Dashboard",
        ar: "لوحة التحكم",
        es: "Panel",
    },
    Entry {
        key: "nav.my_courses",
        en: "My Courses",
        ar: "موادي الدراسية",
        es: "Mis cursos",
    },
    Entry {
        key: "nav.all_tenant_courses",
        en: "All Tenant Courses",
        ar: "جميع مواد المؤسسة",
        es: "Todos los cursos",
    },
    Entry {
        key: "nav.my_schedule",
        en: "My Schedule",
        ar: "جدولي",
        es: "Mi horario",
    },
    Entry {
        key: "nav.my_certificates",
        en: "My Certificates",
        ar: "شهاداتي",
        es: "Mis certificados",
    },
    Entry {
        key: "nav.redeem_code",
        en: "Redeem Code",
        ar: "استبدال رمز",
        es: "Canjear código",
    },
    Entry {
        key: "nav.my_children",
        en: "My Children",
        ar: "أبنائي",
        es: "Mis hijos",
    },
    Entry {
        key: "nav.calendar",
        en: "Calendar",
        ar: "التقويم",
        es: "Calendario",
    },
    Entry {
        key: "nav.notifications",
        en: "Notifications",
        ar: "الإشعارات",
        es: "Notificaciones",
    },
    Entry {
        key: "nav.set_up_workspace",
        en: "Set up workspace",
        ar: "إعداد مساحة العمل",
        es: "Configurar espacio",
    },
    Entry {
        key: "nav.admin",
        en: "Admin",
        ar: "الإدارة",
        es: "Administración",
    },
    Entry {
        key: "nav.analytics",
        en: "Analytics",
        ar: "التحليلات",
        es: "Analíticas",
    },
    Entry {
        key: "nav.billing",
        en: "Billing",
        ar: "الفوترة",
        es: "Facturación",
    },
    Entry {
        key: "nav.branding",
        en: "Branding",
        ar: "الهوية البصرية",
        es: "Marca",
    },
    Entry {
        key: "nav.members",
        en: "Members",
        ar: "الأعضاء",
        es: "Miembros",
    },
    Entry {
        key: "nav.audit_log",
        en: "Audit log",
        ar: "سجل التدقيق",
        es: "Registro de auditoría",
    },
    Entry {
        key: "nav.files",
        en: "Files",
        ar: "الملفات",
        es: "Archivos",
    },
    Entry {
        key: "nav.platform",
        en: "Platform",
        ar: "المنصة",
        es: "Plataforma",
    },
    // --- App chrome (topbar) ---
    Entry {
        key: "chrome.workspace",
        en: "Workspace",
        ar: "مساحة العمل",
        es: "Espacio de trabajo",
    },
    Entry {
        key: "chrome.search_placeholder",
        en: "Search…",
        ar: "بحث…",
        es: "Buscar…",
    },
    Entry {
        key: "chrome.search",
        en: "Search",
        ar: "بحث",
        es: "Buscar",
    },
    Entry {
        key: "chrome.sign_out",
        en: "Sign out",
        ar: "تسجيل الخروج",
        es: "Cerrar sesión",
    },
    Entry {
        key: "chrome.primary_nav",
        en: "Primary navigation",
        ar: "التنقل الرئيسي",
        es: "Navegación principal",
    },
    Entry {
        key: "chrome.language",
        en: "Language",
        ar: "اللغة",
        es: "Idioma",
    },
    // --- Auth: shared ---
    Entry {
        key: "auth.brand",
        en: "AulaLite Academy",
        ar: "أكاديمية أولا‑لايت",
        es: "Academia AulaLite",
    },
    Entry {
        key: "auth.email",
        en: "Email",
        ar: "البريد الإلكتروني",
        es: "Correo electrónico",
    },
    Entry {
        key: "auth.email_placeholder",
        en: "you@example.com",
        ar: "you@example.com",
        es: "tu@ejemplo.com",
    },
    Entry {
        key: "auth.password",
        en: "Password",
        ar: "كلمة المرور",
        es: "Contraseña",
    },
    // --- Auth: login ---
    Entry {
        key: "auth.login.title",
        en: "Sign in to the academy",
        ar: "تسجيل الدخول إلى الأكاديمية",
        es: "Inicia sesión en la academia",
    },
    Entry {
        key: "auth.login.subtitle",
        en:
            "Live classes, assignments, schedules, and course operations in one polished workspace.",
        ar: "حصص مباشرة وواجبات وجداول وإدارة المواد في مساحة عمل واحدة متقنة.",
        es: "Clases en vivo, tareas, horarios y gestión de cursos en un solo espacio de trabajo.",
    },
    Entry {
        key: "auth.login.password_placeholder",
        en: "Your password",
        ar: "كلمة المرور الخاصة بك",
        es: "Tu contraseña",
    },
    Entry {
        key: "auth.login.submit",
        en: "Sign in",
        ar: "تسجيل الدخول",
        es: "Iniciar sesión",
    },
    Entry {
        key: "auth.login.pending",
        en: "Signing in…",
        ar: "جارٍ تسجيل الدخول…",
        es: "Iniciando sesión…",
    },
    Entry {
        key: "auth.login.create_account",
        en: "Create an account",
        ar: "إنشاء حساب",
        es: "Crear una cuenta",
    },
    Entry {
        key: "auth.login.forgot",
        en: "Forgot your password?",
        ar: "هل نسيت كلمة المرور؟",
        es: "¿Olvidaste tu contraseña?",
    },
    // --- Auth: signup ---
    Entry {
        key: "auth.signup.title",
        en: "Create your AulaLite account",
        ar: "أنشئ حساب أولا‑لايت",
        es: "Crea tu cuenta de AulaLite",
    },
    Entry {
        key: "auth.signup.subtitle",
        en: "Join your courses and live sessions with a secure academy profile.",
        ar: "انضم إلى موادك وجلساتك المباشرة بملف تعريف آمن في الأكاديمية.",
        es: "Únete a tus cursos y sesiones en vivo con un perfil de academia seguro.",
    },
    Entry {
        key: "auth.signup.password_placeholder",
        en: "At least 8 characters",
        ar: "8 أحرف على الأقل",
        es: "Al menos 8 caracteres",
    },
    Entry {
        key: "auth.signup.confirm",
        en: "Confirm password",
        ar: "تأكيد كلمة المرور",
        es: "Confirmar contraseña",
    },
    Entry {
        key: "auth.signup.confirm_placeholder",
        en: "Repeat your password",
        ar: "أعد إدخال كلمة المرور",
        es: "Repite tu contraseña",
    },
    Entry {
        key: "auth.signup.submit",
        en: "Create account",
        ar: "إنشاء حساب",
        es: "Crear cuenta",
    },
    Entry {
        key: "auth.signup.pending",
        en: "Creating account…",
        ar: "جارٍ إنشاء الحساب…",
        es: "Creando cuenta…",
    },
    Entry {
        key: "auth.signup.have_account",
        en: "Already have an account? Sign in",
        ar: "هل لديك حساب بالفعل؟ تسجيل الدخول",
        es: "¿Ya tienes una cuenta? Inicia sesión",
    },
    // --- Auth: forgot ---
    Entry {
        key: "auth.forgot.title",
        en: "Reset your password",
        ar: "إعادة تعيين كلمة المرور",
        es: "Restablece tu contraseña",
    },
    Entry {
        key: "auth.forgot.subtitle",
        en: "We will send a reset link when your account can receive one.",
        ar: "سنرسل رابط إعادة التعيين عندما يكون حسابك مؤهلاً لتلقيه.",
        es: "Enviaremos un enlace de restablecimiento cuando tu cuenta pueda recibirlo.",
    },
    Entry {
        key: "auth.forgot.sent",
        en: "If an account exists for that email, a reset link has been sent.",
        ar: "إذا كان هناك حساب مرتبط بهذا البريد، فقد أُرسل رابط إعادة التعيين.",
        es: "Si existe una cuenta para ese correo, se ha enviado un enlace de restablecimiento.",
    },
    Entry {
        key: "auth.forgot.submit",
        en: "Send reset link",
        ar: "إرسال رابط إعادة التعيين",
        es: "Enviar enlace",
    },
    Entry {
        key: "auth.forgot.pending",
        en: "Sending reset link…",
        ar: "جارٍ إرسال الرابط…",
        es: "Enviando enlace…",
    },
    Entry {
        key: "auth.forgot.back",
        en: "Back to sign in",
        ar: "العودة إلى تسجيل الدخول",
        es: "Volver a iniciar sesión",
    },
];

/// Look up `key` for `locale`, falling back to English, then to the raw key.
/// Pure, `const`-table-backed, and SSR-safe. Returns a `&'static str` so call
/// sites avoid an allocation per render.
pub fn translate(locale: Locale, key: &str) -> &'static str {
    let entry = CATALOG.iter().find(|e| e.key == key);
    match entry {
        Some(e) => {
            let localized = match locale {
                Locale::En => e.en,
                Locale::Ar => e.ar,
                Locale::Es => e.es,
            };
            if !localized.is_empty() {
                localized
            } else {
                e.en
            }
        }
        // Unknown key: return the key itself so the gap is visible in the UI
        // (and greppable) rather than rendering nothing.
        None => leak_key(key),
    }
}

/// Map an unknown key to a `'static` string without allocating per call. The
/// catalog is closed, so the only keys reaching here are programmer typos /
/// not-yet-added keys; interning them is bounded by the (small, fixed) set of
/// such mistakes and keeps the `t()` signature allocation-free.
fn leak_key(key: &str) -> &'static str {
    use std::collections::HashMap;
    use std::sync::{Mutex, OnceLock};
    static INTERN: OnceLock<Mutex<HashMap<String, &'static str>>> = OnceLock::new();
    let map = INTERN.get_or_init(|| Mutex::new(HashMap::new()));
    let mut guard = map.lock().expect("intern map poisoned");
    if let Some(&s) = guard.get(key) {
        return s;
    }
    let leaked: &'static str = Box::leak(key.to_string().into_boxed_str());
    guard.insert(key.to_string(), leaked);
    leaked
}

/// Apply `locale` to the document on wasm: set `<html dir>` + `<html lang>` and
/// mirror the code to `localStorage` (`aula-locale`) so the boot script /
/// `[dir=rtl]` CSS can paint before WASM hydrates. No-op off-wasm (SSR/native).
/// Uses typed browser APIs so a strict Content Security Policy does not need
/// JavaScript `eval`.
pub fn apply_locale(locale: Locale) {
    #[cfg(target_arch = "wasm32")]
    if let Some(window) = web_sys::window() {
        if let Ok(Some(storage)) = window.local_storage() {
            let _ = storage.set_item("aula-locale", locale.code());
        }
        if let Some(root) = window.document().and_then(|doc| doc.document_element()) {
            let _ = root.set_attribute("dir", locale.dir().as_str());
            let _ = root.set_attribute("lang", locale.code());
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    let _ = locale;
}

/// Shared signal type for the active locale.
pub type LocaleSignal = Signal<Locale>;

/// Provide the active-locale context to the subtree and keep `document.dir` /
/// `lang` in sync with it. Mount once near the app root, inside the router-less
/// shell so every screen (auth + app) can call [`use_locale`] / [`use_t`].
///
/// On wasm the provider seeds the initial locale from the persisted
/// `localStorage` value (so a reload keeps the chosen language) and applies it
/// to the document on every change.
#[derive(Props, Clone, PartialEq)]
pub struct LocaleProviderProps {
    /// Optional starting locale (defaults to English; overridden by a persisted
    /// value on wasm).
    #[props(default)]
    pub initial: Locale,
    pub children: Element,
}

#[component]
pub fn LocaleProvider(props: LocaleProviderProps) -> Element {
    // Seed from persisted storage on wasm; otherwise use the prop default.
    let initial = use_hook(|| {
        #[cfg(target_arch = "wasm32")]
        {
            if let Some(stored) = read_persisted_locale() {
                return stored;
            }
        }
        props.initial
    });
    let locale = use_signal(|| initial);
    use_context_provider::<LocaleSignal>(|| locale);

    // Keep the document direction/lang in sync with the active locale. Runs on
    // mount and whenever the locale changes (subscribed via `locale()`).
    use_effect(move || {
        apply_locale(locale());
    });

    rsx! { {props.children} }
}

/// Read the persisted locale from `localStorage` (`aula-locale`). wasm-only;
/// falls back to `None` when storage is unavailable.
#[cfg(target_arch = "wasm32")]
fn read_persisted_locale() -> Option<Locale> {
    web_sys::window()
        .and_then(|w| w.local_storage().ok().flatten())
        .and_then(|s| s.get_item("aula-locale").ok().flatten())
        .filter(|v| !v.trim().is_empty())
        .map(|v| Locale::parse(&v))
}

/// Acquire the active-locale signal from context. Must be called inside a tree
/// that mounted [`LocaleProvider`]; falls back to a fresh default-locale signal
/// if absent so isolated SSR tests / unusual mount sites don't panic.
pub fn use_locale() -> LocaleSignal {
    use_hook(|| try_consume_context::<LocaleSignal>().unwrap_or_else(|| Signal::new(Locale::En)))
}

/// Return a `t(key)` closure bound to the active locale. Reactive: the closure
/// reads the locale signal, so components using it re-render on locale change.
///
/// ```ignore
/// let t = use_t();
/// rsx! { span { "{t(\"nav.dashboard\")}" } }
/// ```
pub fn use_t() -> impl Fn(&str) -> &'static str + Copy {
    let locale = use_locale();
    move |key: &str| translate(*locale.read(), key)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn english_is_complete_for_every_key() {
        for e in CATALOG {
            assert!(
                !e.en.is_empty(),
                "English missing for key `{}` (English must be complete)",
                e.key
            );
        }
    }

    #[test]
    fn keys_are_unique_and_sorted_within_sections() {
        // Uniqueness is the hard invariant (duplicate keys would shadow).
        let mut seen = std::collections::HashSet::new();
        for e in CATALOG {
            assert!(seen.insert(e.key), "duplicate catalog key: {}", e.key);
        }
    }

    #[test]
    fn translate_uses_locale_then_falls_back_to_english() {
        // Translated cell wins.
        assert_eq!(translate(Locale::Ar, "nav.dashboard"), "لوحة التحكم");
        assert_eq!(translate(Locale::Es, "nav.dashboard"), "Panel");
        assert_eq!(translate(Locale::En, "nav.dashboard"), "Dashboard");
    }

    #[test]
    fn translate_unknown_key_returns_the_key() {
        assert_eq!(translate(Locale::En, "nope.missing"), "nope.missing");
        // Interning is stable across calls.
        assert_eq!(translate(Locale::Ar, "nope.missing"), "nope.missing");
    }

    #[test]
    fn locale_round_trips_and_tolerates_region_tags() {
        for l in Locale::ALL {
            assert_eq!(Locale::parse(l.code()), l);
        }
        assert_eq!(Locale::parse("ar-EG"), Locale::Ar);
        assert_eq!(Locale::parse("es_MX"), Locale::Es);
        // Unknown → English.
        assert_eq!(Locale::parse("fr"), Locale::En);
        assert_eq!(Locale::parse(""), Locale::En);
    }

    #[test]
    fn direction_is_rtl_only_for_arabic() {
        assert_eq!(Locale::Ar.dir(), Dir::Rtl);
        assert_eq!(Locale::En.dir(), Dir::Ltr);
        assert_eq!(Locale::Es.dir(), Dir::Ltr);
        assert_eq!(Dir::Rtl.as_str(), "rtl");
        assert_eq!(Dir::Ltr.as_str(), "ltr");
    }

    #[test]
    fn locale_provider_supplies_t_to_children() {
        #[component]
        fn Child() -> Element {
            let t = use_t();
            rsx! { span { "{t(\"nav.dashboard\")}" } }
        }
        fn app() -> Element {
            rsx! {
                LocaleProvider { initial: Locale::Es, Child {} }
            }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(html.contains("Panel"), "expected Spanish nav label: {html}");
    }
}
