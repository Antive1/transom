//! Ucet AI nastrojov z ich lokalnych suborov: plan uctu Claude (Pro, Max 5×, Max 20×, Team, Enterprise) a e-mail
//! prihlaseneho uctu Claude a Codex (pri viacerych uctoch je jasne, ku ktoremu patria limity).
//!
//! * Claude: `.claude.json` (`CLAUDE_CONFIG_DIR` alebo `%USERPROFILE%`), z neho **len** `oauthAccount.organizationType`,
//!   `organizationRateLimitTier` a `emailAddress`.
//! * Codex: `auth.json` (`CODEX_HOME` alebo `~/.codex`), z neho **len** narok `email` z `tokens.id_token` (JWT; podpis sa
//!   neoveruje, token sa nikam neposiela ani neuklada).
//!
//! Prihlasovacie udaje sa nikdy nepouzivaju, neukladaju ani nelogu; e-mail sa ukazuje len v paneli AI. Subory sa citaju znova,
//! len ked sa zmenili (cas upravy).
use serde_json::Value;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::SystemTime;

/// Cesta k `.claude.json` (Claude Code ho pri `CLAUDE_CONFIG_DIR` drzi v nom, inak v profile).
fn path() -> Option<PathBuf> {
    if let Some(d) = std::env::var_os("CLAUDE_CONFIG_DIR").filter(|d| !d.is_empty()) {
        return Some(PathBuf::from(d).join(".claude.json"));
    }
    std::env::var_os("USERPROFILE").map(|h| PathBuf::from(h).join(".claude.json"))
}

/// Citatelny nazov planu podla typu organizacie a urovne limitov (`None` = nezname / API bez predplatneho).
pub fn plan_label(org_type: &str, tier: &str) -> Option<String> {
    let t = tier.to_ascii_lowercase();
    Some(match org_type.to_ascii_lowercase().as_str() {
        "claude_max" => {
            if t.contains("20x") {
                "Max 20×".into()
            } else if t.contains("5x") {
                "Max 5×".into()
            } else {
                "Max".into()
            }
        }
        "claude_pro" => "Pro".into(),
        "claude_team" => "Team".into(),
        "claude_enterprise" => "Enterprise".into(),
        _ => return None,
    })
}

/// Plan z obsahu `.claude.json`.
pub fn plan_from_json(text: &str) -> Option<String> {
    claude_from_json(text).0
}

/// (plan, e-mail) z obsahu `.claude.json`.
fn claude_from_json(text: &str) -> (Option<String>, Option<String>) {
    let Ok(v) = serde_json::from_str::<Value>(text) else {
        return (None, None);
    };
    let Some(oa) = v.get("oauthAccount") else {
        return (None, None);
    };
    let plan = oa
        .get("organizationType")
        .and_then(Value::as_str)
        .and_then(|org| {
            let tier = oa
                .get("organizationRateLimitTier")
                .and_then(Value::as_str)
                .unwrap_or("");
            plan_label(org, tier)
        });
    let email = oa
        .get("emailAddress")
        .and_then(Value::as_str)
        .and_then(clean_email);
    (plan, email)
}

/// E-mail na zobrazenie: len rozumna adresa (ziadne riadiace znaky, rozumna dlzka).
fn clean_email(e: &str) -> Option<String> {
    let e = e.trim();
    (e.contains('@') && e.len() <= 120 && !e.chars().any(char::is_control)).then(|| e.to_string())
}

type Cached = Mutex<Option<(SystemTime, (Option<String>, Option<String>))>>;
static CLAUDE: Cached = Mutex::new(None);
static CODEX: Cached = Mutex::new(None);

/// Precita subor `p` cez `parse`, ak sa od posledneho citania zmenil.
fn cached(
    cache: &Cached,
    p: Option<PathBuf>,
    parse: fn(&str) -> (Option<String>, Option<String>),
) -> (Option<String>, Option<String>) {
    let Some(p) = p else { return (None, None) };
    let Ok(modified) = std::fs::metadata(&p).and_then(|m| m.modified()) else {
        return (None, None);
    };
    let mut g = cache.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((t, v)) = g.as_ref() {
        if *t == modified {
            return v.clone();
        }
    }
    let v = std::fs::read_to_string(&p)
        .ok()
        .map(|t| parse(&t))
        .unwrap_or((None, None));
    *g = Some((modified, v.clone()));
    v
}

/// Aktualny plan uctu Claude (z lokalneho suboru Claude Code), ak je znamy.
pub fn claude_plan() -> Option<String> {
    cached(&CLAUDE, path(), claude_from_json).0
}

/// E-mail prihlaseneho uctu Claude Code, ak je znamy.
pub fn claude_email() -> Option<String> {
    cached(&CLAUDE, path(), claude_from_json).1
}

fn codex_auth_path() -> Option<PathBuf> {
    if let Some(d) = std::env::var_os("CODEX_HOME").filter(|d| !d.is_empty()) {
        return Some(PathBuf::from(d).join("auth.json"));
    }
    std::env::var_os("USERPROFILE").map(|h| PathBuf::from(h).join(".codex").join("auth.json"))
}

/// Base64url bez doplnku (cast JWT).
fn b64url(s: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(s.len() * 3 / 4);
    let (mut buf, mut bits) = (0u32, 0u32);
    for c in s.bytes() {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'-' | b'+' => 62,
            b'_' | b'/' => 63,
            b'=' => break,
            _ => return None,
        } as u32;
        buf = buf << 6 | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buf >> bits) as u8);
            buf &= (1 << bits) - 1;
        }
    }
    Some(out)
}

/// (plan, e-mail) z `auth.json` Codexu: len narok `email` z `tokens.id_token` (plan hlasia relacie Codexu).
fn codex_from_json(text: &str) -> (Option<String>, Option<String>) {
    let email = serde_json::from_str::<Value>(text)
        .ok()
        .and_then(|v| {
            v.get("tokens")?
                .get("id_token")?
                .as_str()
                .map(str::to_string)
        })
        .and_then(|jwt| {
            let payload = jwt.split('.').nth(1)?.to_string();
            let claims: Value = serde_json::from_slice(&b64url(&payload)?).ok()?;
            claims.get("email")?.as_str().and_then(clean_email)
        });
    (None, email)
}

/// E-mail prihlaseneho uctu Codex (ChatGPT), ak je znamy.
pub fn codex_email() -> Option<String> {
    cached(&CODEX, codex_auth_path(), codex_from_json).1
}

fn home_dir(var: &str, dot: &str) -> Option<PathBuf> {
    if let Some(d) = std::env::var_os(var).filter(|d| !d.is_empty()) {
        return Some(PathBuf::from(d));
    }
    std::env::var_os("USERPROFILE").map(|h| PathBuf::from(h).join(dot))
}

/// (plan, e-mail) z `~/.grok/auth.json`: zaznamy podla rozsahu prihlasenia; prednost ma `https://auth.x.ai::...` (OIDC,
/// SuperGrok), potom stary `.../sign-in`. Cita sa len `email` a `auth_mode` (tokeny nie).
fn grok_from_json(text: &str) -> (Option<String>, Option<String>) {
    let Ok(Value::Object(m)) = serde_json::from_str::<Value>(text) else {
        return (None, None);
    };
    let pick = m
        .iter()
        .find(|(k, _)| k.starts_with("https://auth.x.ai::"))
        .or_else(|| m.iter().find(|(k, _)| k.contains("/sign-in")))
        .map(|(_, v)| v);
    let Some(e) = pick else {
        return (None, None);
    };
    let plan = e
        .get("auth_mode")
        .and_then(Value::as_str)
        .filter(|a| a.eq_ignore_ascii_case("oidc"))
        .map(|_| "SuperGrok".to_string());
    let email = e.get("email").and_then(Value::as_str).and_then(clean_email);
    (plan, email)
}

static GROK: Cached = Mutex::new(None);

/// E-mail a plan prihlaseneho uctu Grok CLI.
pub fn grok_account() -> (Option<String>, Option<String>) {
    cached(
        &GROK,
        home_dir("GROK_HOME", ".grok").map(|d| d.join("auth.json")),
        grok_from_json,
    )
}

/// E-mail prihlaseneho uctu poskytovatela (ak ho jeho lokalne subory prezradia).
pub fn email_for(p: crate::Provider) -> Option<String> {
    match p {
        crate::Provider::ClaudeCode => claude_email(),
        crate::Provider::Codex => codex_email(),
        crate::Provider::Grok => grok_account().1,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plan_names_follow_the_organization_type_and_tier() {
        assert_eq!(
            plan_label("claude_max", "default_claude_max_5x").as_deref(),
            Some("Max 5×")
        );
        assert_eq!(
            plan_label("claude_max", "default_claude_max_20x").as_deref(),
            Some("Max 20×")
        );
        assert_eq!(plan_label("claude_max", "").as_deref(), Some("Max"));
        assert_eq!(
            plan_label("claude_pro", "default_claude_ai").as_deref(),
            Some("Pro")
        );
        assert_eq!(plan_label("claude_team", "").as_deref(), Some("Team"));
        assert_eq!(plan_label("api", ""), None);
    }

    #[test]
    fn emails_come_from_the_account_fields_and_the_codex_id_token() {
        let json = r#"{"oauthAccount":{"emailAddress":"a@b.cz","organizationType":"claude_pro"}}"#;
        assert_eq!(
            claude_from_json(json),
            (Some("Pro".into()), Some("a@b.cz".into()))
        );
        // JWT s narokom email (podpis sa neoveruje)
        let payload = "eyJlbWFpbCI6InVzZXJAZXhhbXBsZS5jb20iLCJuYW1lIjoiWCJ9"; // {"email":"user@example.com","name":"X"}
        let auth = format!(r#"{{"tokens":{{"id_token":"h.{payload}.s","access_token":"t"}}}}"#);
        assert_eq!(
            codex_from_json(&auth).1.as_deref(),
            Some("user@example.com")
        );
        assert_eq!(codex_from_json(r#"{"tokens":{"id_token":"bad"}}"#).1, None);
        assert_eq!(b64url("YWJj").unwrap(), b"abc");
    }

    #[test]
    fn grok_account_prefers_the_oidc_sign_in() {
        let grok = r#"{"https://accounts.x.ai/sign-in":{"key":"k1","email":"old@x.ai","auth_mode":"session"},
            "https://auth.x.ai::abc":{"key":"k2","email":"me@x.ai","auth_mode":"oidc"}}"#;
        assert_eq!(
            grok_from_json(grok),
            (Some("SuperGrok".into()), Some("me@x.ai".into()))
        );
    }

    #[test]
    fn only_the_plan_fields_are_used_from_the_file() {
        let json = r#"{"numStartups":3,"oauthAccount":{"emailAddress":"x@y.z","displayName":"N","organizationType":"claude_max","organizationRateLimitTier":"default_claude_max_5x"}}"#;
        assert_eq!(plan_from_json(json).as_deref(), Some("Max 5×"));
        assert_eq!(plan_from_json(r#"{"numStartups":1}"#), None);
        assert_eq!(plan_from_json("not json"), None);
    }
}
