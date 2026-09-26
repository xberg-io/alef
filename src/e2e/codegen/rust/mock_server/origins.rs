//! Origin-substitution source fragment shared by both generated Rust mock servers.
//!
//! `allow_subdomains` / `stay_on_domain` / external-link fixtures are exercised by
//! links INSIDE served content, not by the URL a harness hands the library under
//! test -- the mock server is the only component that knows its own port, so it
//! substitutes a small token vocabulary into response bodies and header values
//! immediately before responding. This needs no codegen or harness changes: a
//! fixture simply writes `{{mock_origin}}`, `{{mock_alt_origin}}`,
//! `{{mock_sub_origin}}`, or `{{mock_foreign_origin}}` into a body or header value
//! wherever it wants a same-origin, alternate-hostname, subdomain, or genuinely
//! external link.
//!
//! Every type here is fully qualified (`std::sync::OnceLock`, not `OnceLock`) so
//! this fragment can be spliced into either generated file without also having to
//! keep that file's `use` list in sync.

const ORIGINS_SOURCE: &str = r####"// ---------------------------------------------------------------------------
// Origin substitution
// ---------------------------------------------------------------------------
//
// Same PORT throughout every token -- only the HOSTNAME varies. `localhost` and
// `127.0.0.1` are distinct hostnames to every URL-policy implementation and both
// resolve everywhere, so cross-origin / subdomain / external-link fixtures ship
// with zero DNS exposure for the two primary tokens. A different port would be
// wrong here: these fixtures must differ in host with the port held constant, or
// they would pass for the wrong reason.

/// Per-listener origin tokens, resolved once when that listener starts and
/// substituted into every response it serves.
#[derive(Clone, Debug)]
struct Origins {
    /// `{{mock_origin}}` -> `http://127.0.0.1:<port>`, this same listener.
    origin: String,
    /// `{{mock_alt_origin}}` -> `http://<alt_host_base>:<port>`, this same listener
    /// under a different hostname (`<alt_host_base>` defaults to `localhost`; see
    /// [`alt_host_base_from_env`]).
    alt_origin: String,
    /// `{{mock_sub_origin}}` -> `http://sub.<alt_host_base>:<port>`, this same
    /// listener under a subdomain of the alternate hostname.
    sub_origin: String,
    /// `{{mock_foreign_origin}}` -> `http://127.0.0.2:<port>`. Deliberately NOT
    /// bound by any listener: a connection here refuses, which is the correct
    /// outcome for a fixture asserting "genuinely external, must not be fetched".
    foreign_origin: String,
}

impl Origins {
    /// The four `(token, value)` pairs substituted into served content.
    fn token_pairs(&self) -> [(&'static str, &str); 4] {
        [
            ("{{mock_origin}}", self.origin.as_str()),
            ("{{mock_alt_origin}}", self.alt_origin.as_str()),
            ("{{mock_sub_origin}}", self.sub_origin.as_str()),
            ("{{mock_foreign_origin}}", self.foreign_origin.as_str()),
        ]
    }

    /// Substitute origin tokens into a header value (e.g. `Location`, `Refresh`).
    fn substitute_str(&self, value: &str) -> String {
        let mut out = value.to_string();
        for (token, replacement) in self.token_pairs() {
            out = out.replace(token, replacement);
        }
        out
    }

    /// Substitute origin tokens into response body bytes.
    ///
    /// Works at the byte level rather than through `String`/UTF-8 conversion: a
    /// binary fixture body (an image, a compressed payload) that happens not to be
    /// valid UTF-8 must pass through untouched, and a lossy UTF-8 round-trip would
    /// silently corrupt it even when it contains no token at all.
    fn substitute_bytes(&self, bytes: Vec<u8>) -> Vec<u8> {
        let mut current = bytes;
        for (token, replacement) in self.token_pairs() {
            current = replace_all_bytes(&current, token.as_bytes(), replacement.as_bytes());
        }
        current
    }
}

/// Byte-level substring replace. Used instead of a `String`-based `.replace()` so
/// binary bodies without a UTF-8-valid representation are never touched by a lossy
/// conversion (see [`Origins::substitute_bytes`]).
fn replace_all_bytes(haystack: &[u8], needle: &[u8], replacement: &[u8]) -> Vec<u8> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return haystack.to_vec();
    }
    let mut out = Vec::with_capacity(haystack.len());
    let mut index = 0;
    while index < haystack.len() {
        if haystack[index..].starts_with(needle) {
            out.extend_from_slice(replacement);
            index += needle.len();
        } else {
            out.push(haystack[index]);
            index += 1;
        }
    }
    out
}

/// The base hostname `{{mock_alt_origin}}` resolves directly to and `{{mock_sub_origin}}`
/// resolves against with a `sub.` prefix, e.g. `sub.localhost` for the default base
/// `localhost`. Configurable via `ALEF_MOCK_ALT_HOST` (the env var every harness spawn site
/// sets from a consumer's `[crates.e2e] alt_host` config before launching this binary),
/// falling back to `"localhost"` when unset or empty.
fn alt_host_base_from_env() -> String {
    std::env::var("ALEF_MOCK_ALT_HOST").ok().filter(|value| !value.is_empty()).unwrap_or_else(|| "localhost".to_string())
}

/// RFC 6761 makes `*.localhost` resolve to loopback, and that holds on macOS, glibc
/// >= 2.36 / systemd-resolved, and Windows 10 1803+ -- but NOT on musl/Alpine or
/// minimal containers. Probing here means a subdomain fixture fails with a DNS error
/// naming the alias on those platforms, instead of an inscrutable 404 from a listener
/// nothing is actually bound to.
fn sub_host_resolves_to_loopback(sub_host: &str) -> bool {
    use std::net::ToSocketAddrs;
    match (sub_host, 0u16).to_socket_addrs() {
        Ok(addrs) => {
            let mut resolved_any = false;
            for addr in addrs {
                resolved_any = true;
                if !addr.ip().is_loopback() {
                    return false;
                }
            }
            resolved_any
        }
        Err(_) => false,
    }
}

/// Choose the real sub-host or a poison host, given whether `sub_host` was already
/// found to resolve to loopback. Split out from [`resolve_origins`] so the branch
/// itself -- not the platform-dependent DNS probe that feeds it -- is directly
/// testable with a known boolean instead of a real (and platform-variable) lookup.
/// `.invalid` is RFC 2606-reserved and never resolvable, so a poisoned
/// `{{mock_sub_origin}}` fails loudly (a DNS error naming the alias) instead of
/// silently 404ing.
fn sub_origin_or_poison(sub_host: &str, port: u16, resolves_to_loopback: bool) -> String {
    if resolves_to_loopback {
        return format!("http://{sub_host}:{port}");
    }
    eprintln!(
        "mock-server: {sub_host} does not resolve to loopback on this platform (musl/Alpine \
         and minimal containers commonly lack *.localhost support); substituting a poison \
         origin so {{{{mock_sub_origin}}}} fixtures fail with a clear DNS error instead of a \
         silent 404"
    );
    format!("http://alef-unresolvable-alt-host.invalid:{port}")
}

/// Resolve the origin set for a listener bound to `port`.
fn resolve_origins(port: u16) -> Origins {
    let alt_host_base = alt_host_base_from_env();
    let sub_host = format!("sub.{alt_host_base}");
    let resolves_to_loopback = sub_host_resolves_to_loopback(&sub_host);
    Origins {
        origin: format!("http://127.0.0.1:{port}"),
        alt_origin: format!("http://{alt_host_base}:{port}"),
        sub_origin: sub_origin_or_poison(&sub_host, port, resolves_to_loopback),
        foreign_origin: format!("http://127.0.0.2:{port}"),
    }
}

"####;

pub(super) fn render_origins_source() -> &'static str {
    ORIGINS_SOURCE
}
