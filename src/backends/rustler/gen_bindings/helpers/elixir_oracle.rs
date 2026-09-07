//! Executable oracles that run generated Elixir through the REAL `elixir` toolchain and assert on
//! the VALUES it produces, not on the text alef emitted.
//!
//! Parse-clean is not correct here, and that is not a slogan — it is the specific reason this file
//! exists. An unescaped `#{...}` inside a generated string literal or quoted atom PARSES. So does
//! the escaped form. `Code.string_to_quoted` returns `{:ok, _}` for both, which is why the
//! doc-snippet validator (`crate::snippets::validators::elixir`) — the only Elixir check this repo
//! had — could not have caught the defect these lanes cover. The two forms differ only once the
//! module is compiled: the unescaped one evaluates the interpolation, so the generated file runs
//! whatever a dependency wrote in a `#[serde(rename = "...")]`, and the literal never reaches the
//! wire at all. Verified on Elixir 1.20.4, both directions.
//!
//! Every lane below is therefore an EVALUATION lane: it compiles the generated module and reads
//! back what its functions return, plus whether a canary file the payload would have written
//! exists. [`parse_alone_cannot_distinguish_an_interpolating_literal`] is the executable proof of
//! the paragraph above, kept as a lane so the methodology claim is checked rather than asserted in
//! a comment.
//!
//! **Requires `elixir` on PATH.** Every lane is `#[ignore]`d so a machine without it cannot report
//! a green having validated nothing, and when a lane IS selected a missing interpreter panics
//! rather than skipping: selecting an ignored lane is an explicit request to run it. `gate.rs`
//! holds the other half — that something still selects them.

use super::conversions::gen_elixir_enum_module;
use crate::backends::rustler::elixir_escape::{elixir_atom_body, escape_elixir_string_literal};
use crate::backends::rustler::gen_bindings::public_api_args::emit_tagged_enum_encoder;
use crate::core::ir::{EnumDef, EnumVariant, FieldDef, PrimitiveType, TypeRef};
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

mod gate;

/// Elixir module prefix for the generated enum module under test.
const APP_MODULE: &str = "AlefOracle";

/// Payload for the escape round-trip lane: an interpolation opener plus the other two characters
/// that need escaping. Fed straight to the escaping primitives, not through any emitter -- this
/// lane is about the primitives themselves.
const ESCAPE_PROBE_PAYLOAD: &str = "lit#{1 + 1}era\"l\\end";

/// The in-lane negative control, embedded with NO escaping at all. Deliberately carries no `"`
/// or `\` so the unescaped form still PARSES: the only thing separating it from the escaped
/// payload is that Elixir evaluates its interpolation, which is exactly the distinction the lane
/// has to be able to draw. Without it the lane could pass while evaluating nothing. ~keep
const ESCAPE_PROBE_CONTROL: &str = "ctl#{1 + 1}end";

/// Every `ALEF|<key>|<verdict>` line [`PROBE_SCRIPT`] emits. Asserted as a count so a probe that
/// silently checked less than it claims fails instead of passing on the lines it did print. ~keep
const PROBE_KEYS: &[&str] = &[
    "plain",
    "interpolated",
    "quoted",
    "controls",
    "digit_wire",
    "atom_spelling",
    "tag",
    "enc_wire",
    "enc_field_key",
    "escaped_string",
    "escaped_atom",
    "unescaped_control",
    "canary",
];

/// The probe carries no Rust-side interpolation on purpose. Every expected value is read from a
/// file written as raw bytes, so nothing in this test has to escape a string for Elixir — which
/// is the very function under test, and would make the oracle circular. ~keep
const PROBE_SCRIPT: &str = r##"defmodule Probe do
  def read(name), do: File.read!(Path.join("expected", name))

  def check(key, actual) do
    expected = read(key)

    verdict =
      if actual == expected do
        "MATCH"
      else
        "MISMATCH actual=" <> inspect(actual) <> " expected=" <> inspect(expected)
      end

    IO.puts("ALEF|" <> key <> "|" <> verdict)
  end
end

Code.compile_file("enum_module.ex")
Code.compile_file("encoder.ex")
Code.compile_file("escape_probe.ex")

marker = AlefOracle.Marker
Probe.check("plain", marker.wire_value(marker.plain()))
Probe.check("interpolated", marker.wire_value(marker.interpolated()))
Probe.check("quoted", marker.wire_value(marker.quoted()))
Probe.check("controls", marker.wire_value(marker.controls()))
Probe.check("digit_wire", marker.wire_value(marker.digit_wire()))
Probe.check("atom_spelling", Atom.to_string(marker.interpolated()))

unit = AlefOracleEncoder.encode_action(:unit_variant)
[{tag_key, tag_value}] = Map.to_list(unit)
Probe.check("tag", tag_key)
Probe.check("enc_wire", tag_value)

data = AlefOracleEncoder.encode_action({:data_variant, %{full_page: true}})
[field_key] = data |> Map.keys() |> Enum.reject(fn key -> key == tag_key end)
Probe.check("enc_field_key", field_key)

Probe.check("escaped_string", AlefEscapeProbe.escaped_string())
Probe.check("escaped_atom", Atom.to_string(AlefEscapeProbe.escaped_atom()))

IO.puts(
  "ALEF|unescaped_control|" <>
    if AlefEscapeProbe.unescaped_string() == Probe.read("unescaped_control") do
      "LITERAL"
    else
      "EVALUATED"
    end
)

canary = String.trim(Probe.read("canary_path"))
IO.puts("ALEF|canary|" <> if(File.exists?(canary), do: "EXISTS", else: "ABSENT"))
"##;

/// Proves the claim the whole file rests on, without involving alef at all: the same payload,
/// escaped and unescaped, both parse; only evaluation separates them.
const PARSE_VS_EVAL_SCRIPT: &str = r##"payload = "interp" <> <<0x23>> <> "{1 + 1}end"

for {label, body} <- [{"UNESCAPED", payload}, {"ESCAPED", String.replace(payload, "#", "\\#")}] do
  source = "defmodule Probe" <> label <> " do\n  def value, do: \"" <> body <> "\"\nend\n"

  parses =
    case Code.string_to_quoted(source) do
      {:ok, _} -> "PARSES"
      {:error, _} -> "PARSE_ERROR"
    end

  [{module, _}] = Code.compile_string(source)
  literal = if module.value() == payload, do: "LITERAL", else: "EVALUATED"
  IO.puts("ALEF|" <> label <> "|" <> parses <> "|" <> literal)
end
"##;

/// A unit enum whose every `#[serde(rename = "...")]` is hostile in a different way: an Elixir
/// interpolation that would write `canary`, a value carrying `"` and `\`, control characters, and
/// a wire name that is not a legal bare atom.
fn hostile_unit_enum(canary: &Path) -> EnumDef {
    let renamed = |name: &str, rename: String| EnumVariant {
        name: name.to_owned(),
        serde_rename: Some(rename),
        ..EnumVariant::default()
    };
    EnumDef {
        name: "Marker".to_owned(),
        variants: vec![
            EnumVariant {
                name: "Plain".to_owned(),
                ..EnumVariant::default()
            },
            renamed("Interpolated", interpolation_payload(canary)),
            renamed("Quoted", "he\"re\\and".to_owned()),
            renamed("Controls", "nl\nnul\u{0}tab\t".to_owned()),
            renamed("DigitWire", "123".to_owned()),
        ],
        ..EnumDef::default()
    }
}

/// A serde-tagged enum whose TAG carries the interpolation payload and whose one data variant
/// renames a field to another hostile wire name. The tag was interpolated into
/// `%{"{tag}" => ...}` with no escaping whatsoever before this branch.
fn hostile_tagged_enum(canary: &Path) -> EnumDef {
    EnumDef {
        name: "Action".to_owned(),
        serde_tag: Some(interpolation_payload(canary)),
        variants: vec![
            EnumVariant {
                name: "UnitVariant".to_owned(),
                ..EnumVariant::default()
            },
            EnumVariant {
                name: "DataVariant".to_owned(),
                fields: vec![FieldDef {
                    name: "full_page".to_owned(),
                    ty: TypeRef::Primitive(PrimitiveType::Bool),
                    serde_rename: Some("fu\"ll\\Pa#{1}ge".to_owned()),
                    ..FieldDef::default()
                }],
                ..EnumVariant::default()
            },
        ],
        ..EnumDef::default()
    }
}

/// The payload a dependency could put in a serde attribute. If it is copied into generated Elixir
/// unescaped it does not fail to compile — it writes `canary` while the module compiles.
fn interpolation_payload(canary: &Path) -> String {
    format!("wire#{{File.write!(\"{}\", \"pwned\")}}end", canary.display())
}

/// Build the fixture directory, run [`PROBE_SCRIPT`] in it, and return the probe's stdout.
fn run_probe() -> String {
    let dir = scratch_dir("probe");
    let expected = dir.join("expected");
    std::fs::create_dir_all(&expected).expect("create oracle fixture directory");
    let canary = dir.join("canary.txt");

    let unit_enum = hostile_unit_enum(&canary);
    let tagged_enum = hostile_tagged_enum(&canary);

    write(
        &dir.join("enum_module.ex"),
        &gen_elixir_enum_module(&unit_enum, APP_MODULE),
    );
    let clauses = emit_tagged_enum_encoder(&tagged_enum);
    assert!(
        clauses.contains("defp encode_action("),
        "the encoder fixture produced no clauses, so the encoder lanes below would check \
         nothing; got:\n{clauses}"
    );
    // The encoder's clauses are `defp`. Promoting them to `def` inside a module of our own is
    // what makes them callable; nothing else about the generated text is touched. ~keep
    write(
        &dir.join("encoder.ex"),
        &format!(
            "defmodule AlefOracleEncoder do\n{}\nend\n",
            clauses.replace("  defp ", "  def ")
        ),
    );

    let escaped = escape_elixir_string_literal(ESCAPE_PROBE_PAYLOAD);
    let atom = elixir_atom_body(ESCAPE_PROBE_PAYLOAD);
    let escape_probe = [
        "defmodule AlefEscapeProbe do".to_owned(),
        format!("  def escaped_string, do: \"{escaped}\""),
        format!("  def escaped_atom, do: :{atom}"),
        format!("  def unescaped_string, do: \"{ESCAPE_PROBE_CONTROL}\""),
        "end".to_owned(),
        String::new(),
    ]
    .join("\n");
    write(&dir.join("escape_probe.ex"), &escape_probe);

    write(&expected.join("canary_path"), &canary.display().to_string());
    write(&expected.join("plain"), "Plain");
    write(&expected.join("interpolated"), &interpolation_payload(&canary));
    write(&expected.join("quoted"), "he\"re\\and");
    write(&expected.join("controls"), "nl\nnul\u{0}tab\t");
    write(&expected.join("digit_wire"), "123");
    write(&expected.join("atom_spelling"), "interpolated");
    write(&expected.join("tag"), &interpolation_payload(&canary));
    write(&expected.join("enc_wire"), "UnitVariant");
    write(&expected.join("enc_field_key"), "fu\"ll\\Pa#{1}ge");
    write(&expected.join("escaped_string"), ESCAPE_PROBE_PAYLOAD);
    write(&expected.join("escaped_atom"), ESCAPE_PROBE_PAYLOAD);
    write(&expected.join("unescaped_control"), ESCAPE_PROBE_CONTROL);
    write(&dir.join("probe.exs"), PROBE_SCRIPT);

    let stdout = run_elixir(&dir, "probe.exs");
    let reported = stdout.lines().filter(|line| line.starts_with("ALEF|")).count();
    assert_eq!(
        reported,
        PROBE_KEYS.len(),
        "the probe must report every check it claims to run; got:\n{stdout}"
    );
    let _ = std::fs::remove_dir_all(&dir);
    stdout
}

/// Assert one `ALEF|<key>|MATCH` line is present, naming the key so a failure says which value
/// came back wrong rather than dumping the whole transcript unlabelled.
fn assert_match(stdout: &str, key: &str) {
    let needle = format!("ALEF|{key}|MATCH");
    assert!(
        stdout.lines().any(|line| line == needle),
        "expected `{needle}` from the real Elixir runtime; full probe output:\n{stdout}"
    );
}

fn write(path: &Path, contents: &str) {
    std::fs::write(path, contents).unwrap_or_else(|error| panic!("write {}: {error}", path.display()));
}

fn scratch_dir(label: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock is after the unix epoch")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("alef_elixir_oracle_{label}_{}_{nanos}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap_or_else(|error| panic!("create {}: {error}", dir.display()));
    dir
}

/// Run one `.exs` script with the real interpreter, from `dir`.
///
/// A missing `elixir` panics. These lanes are `#[ignore]`d, so reaching this function at all means
/// a runner explicitly asked for them with `--ignored`; answering that request by returning
/// quietly is how a suite reports green having validated nothing. ~keep
fn run_elixir(dir: &Path, script: &str) -> String {
    let output = match crate::core::tool_command("elixir")
        .arg(script)
        .current_dir(dir)
        .output()
    {
        Ok(output) => output,
        Err(error) if error.kind() == ErrorKind::NotFound => panic!(
            "`elixir` is not on PATH, but this lane was explicitly selected (it is #[ignore]d and \
             runs only under --ignored). Install Elixir or do not select these lanes; skipping \
             would report a pass having evaluated nothing. Underlying error: {error}"
        ),
        Err(error) => panic!("failed to run `elixir {script}`: {error}"),
    };
    assert!(
        output.status.success(),
        "`elixir {script}` failed ({}).\nstdout:\n{}\nstderr:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).into_owned()
}

// ---------------------------------------------------------------------------
// Lanes
// ---------------------------------------------------------------------------

/// The generated enum module, compiled and CALLED. `wire_value(accessor())` must return each
/// variant's serde wire name byte for byte — which covers three things at once: the escaping is
/// right (the value is the literal, not an evaluated interpolation), the atom spelling agrees
/// between the accessor and `wire_value/1`'s clause head (before this branch the composition
/// raised `FunctionClauseError`), and control characters survive the round trip.
#[test]
#[ignore = "evaluates generated Elixir through the real `elixir` toolchain; \
            run with `cargo test --lib elixir_oracle -- --ignored`"]
fn generated_enum_module_returns_literal_wire_values() {
    let stdout = run_probe();
    for key in [
        "plain",
        "interpolated",
        "quoted",
        "controls",
        "digit_wire",
        "atom_spelling",
    ] {
        assert_match(&stdout, key);
    }
}

/// The tagged-enum encoder, compiled and CALLED. The serde tag and the renamed field key it puts
/// into the encoded map must be the literal strings, not whatever their `#{...}` evaluated to.
#[test]
#[ignore = "evaluates generated Elixir through the real `elixir` toolchain; \
            run with `cargo test --lib elixir_oracle -- --ignored`"]
fn tagged_enum_encoder_emits_literal_tag_and_field_keys() {
    let stdout = run_probe();
    for key in ["tag", "enc_wire", "enc_field_key"] {
        assert_match(&stdout, key);
    }
}

/// The security half, stated as a side effect rather than a value: compiling the two generated
/// modules must not run the `File.write!` a serde attribute asked for. `ABSENT` is the whole
/// assertion — the canary path is handed to Elixir as raw bytes in a file, so this cannot pass by
/// checking the wrong path.
#[test]
#[ignore = "evaluates generated Elixir through the real `elixir` toolchain; \
            run with `cargo test --lib elixir_oracle -- --ignored`"]
fn compiling_generated_modules_executes_no_payload_from_a_serde_attribute() {
    let stdout = run_probe();
    assert!(
        stdout.lines().any(|line| line == "ALEF|canary|ABSENT"),
        "a `#{{File.write!(..)}}` payload in a serde attribute ran while the generated modules \
         compiled; full probe output:\n{stdout}"
    );
}

/// The escaping primitives themselves, EVALUATED rather than pattern-matched.
///
/// The property is not "does `#{` appear in the output". The correct escaped form is `\#{`, which
/// still contains `#{` as a substring, so a `!contains("#{")` assertion rejects correct output --
/// a false negative that fires on the fix instead of on the bug. (The Rust-side unit test in
/// `elixir_escape::tests` asserted exactly that, and this lane is what it now defers to.) The
/// property is whether the literal INTERPOLATES when Elixir evaluates it, and only Elixir can
/// answer that.
///
/// So the probe module carries three definitions of the same shape: an escaped string literal, an
/// escaped quoted atom, and -- as the in-lane negative control -- an UNESCAPED copy. The first two
/// must evaluate back to the exact payload; the third must NOT, or the lane is evaluating
/// something that could not have interpolated in the first place and proves nothing. ~keep
#[test]
#[ignore = "evaluates escaped Elixir literals through the real `elixir` toolchain; \
            run with `cargo test --lib elixir_oracle -- --ignored`"]
fn escaped_literals_round_trip_and_an_unescaped_one_does_not() {
    let stdout = run_probe();
    assert_match(&stdout, "escaped_string");
    assert_match(&stdout, "escaped_atom");
    assert!(
        stdout.lines().any(|line| line == "ALEF|unescaped_control|EVALUATED"),
        "the unescaped control must INTERPOLATE when evaluated. If it came back LITERAL, the \
         payload it carries cannot interpolate at all, so the two MATCH verdicts above are \
         consistent with an escaper that does nothing; full probe output:\n{stdout}"
    );
}

/// Why every lane above evaluates instead of parsing. Involves no alef code: it builds both forms
/// of the same payload in Elixir and reports, for each, whether it parses and whether the value it
/// produces is the literal. If this ever reports `UNESCAPED|PARSES|LITERAL`, interpolation stopped
/// being a hazard and the lanes above are guarding nothing.
#[test]
#[ignore = "runs the real `elixir` toolchain; \
            run with `cargo test --lib elixir_oracle -- --ignored`"]
fn parse_alone_cannot_distinguish_an_interpolating_literal() {
    let dir = scratch_dir("parse_vs_eval");
    write(&dir.join("probe.exs"), PARSE_VS_EVAL_SCRIPT);
    let stdout = run_elixir(&dir, "probe.exs");
    let _ = std::fs::remove_dir_all(&dir);

    assert!(
        stdout.lines().any(|line| line == "ALEF|UNESCAPED|PARSES|EVALUATED"),
        "the unescaped payload must still PARSE (so a parse gate would pass it) while EVALUATING \
         to something other than the literal; got:\n{stdout}"
    );
    assert!(
        stdout.lines().any(|line| line == "ALEF|ESCAPED|PARSES|LITERAL"),
        "the escaped payload must parse AND evaluate to the literal; got:\n{stdout}"
    );
}
