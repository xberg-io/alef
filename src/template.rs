//! Template-context construction for the minijinja 3 value model.
//!
//! ~keep minijinja 3 removed `Value::from_serialize` and made `context!` build each value with
//! `Value::from` (no serialization), so passing a borrowed or non-primitive value inline no
//! longer compiles. Alef's ~3000 `context!` call sites freely pass borrowed data. These helpers
//! serialize by reference instead, so the call sites stay unchanged; `configure_env` carries the
//! block-trimming syntax the templates are authored against.

use minijinja::Environment;
use minijinja::Value;
use minijinja::syntax::SyntaxConfig;
use minijinja::value::Serde;
use serde::Serialize;

/// Serialize any value, borrowed or owned, into a minijinja [`Value`].
pub fn to_value<T: Serialize>(value: T) -> Value {
    Value::from(Serde(value))
}

/// Apply the block-trimming syntax every generated template is authored against.
pub fn configure_env(env: &mut Environment<'_>) {
    let syntax = SyntaxConfig::builder()
        .trim_blocks(true)
        .lstrip_blocks(true)
        .keep_trailing_newline(true)
        .build()
        .expect("the default block-trimming syntax config is valid");
    env.set_syntax(syntax);
}

/// Build a minijinja context from `key` / `key => value` pairs, serializing each value by
/// reference so borrowed data is accepted exactly as minijinja 2's `context!` accepted it.
#[macro_export]
macro_rules! alef_context {
    ($($key:ident $(=> $value:expr)?),* $(,)?) => {{
        let mut __alef_context = ::std::collections::BTreeMap::<&'static str, ::minijinja::Value>::new();
        $(
            __alef_context.insert(
                stringify!($key),
                $crate::template::to_value(&$crate::__alef_context_value!($key $(=> $value)?)),
            );
        )*
        ::minijinja::Value::from(::minijinja::value::Serde(__alef_context))
    }};
}

#[macro_export]
#[doc(hidden)]
macro_rules! __alef_context_value {
    ($key:ident) => {
        $key
    };
    ($key:ident => $value:expr) => {
        $value
    };
}
