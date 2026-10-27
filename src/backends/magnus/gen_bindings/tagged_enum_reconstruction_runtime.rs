use magnus::{IntoValue, TryConvert};
use serde_json::{Value as Json, json};

fn check<T>(ruby: &Ruby, wire: Json, class: &str) -> magnus::Value
where
    T: serde::de::DeserializeOwned + serde::Serialize + IntoValue,
{
    let value: T = serde_json::from_value(wire.clone()).unwrap();
    let result = value.into_value_with(ruby);
    let actual: String = result.class().funcall("name", ()).unwrap();
    assert_eq!(actual, class);
    if class == "Hash" {
        let serialized: String = result.funcall("to_json", ()).unwrap();
        assert_eq!(serde_json::from_str::<Json>(&serialized).unwrap(), wire);
    }
    result
}

fn record<T>(ruby: &Ruby, adjacent: bool, class: &str, roundtrip: bool)
where
    T: serde::de::DeserializeOwned + serde::Serialize + IntoValue + TryConvert,
{
    let wire = if adjacent {
        json!({"kind":"Record", "data":{"label":"retained"}})
    } else {
        json!({"kind":"Record", "label":"retained"})
    };
    let result = check::<T>(ruby, wire.clone(), class);
    if roundtrip {
        let payload: magnus::Value = result.funcall("value", ()).unwrap();
        let payload = <&Payload as TryConvert>::try_convert(payload).unwrap();
        assert_eq!(payload.label, "retained");
        let restored = T::try_convert(result).unwrap();
        assert_eq!(serde_json::to_value(restored).unwrap(), wire);
    }
}

fn fallbacks<T>(ruby: &Ruby, adjacent: bool)
where
    T: serde::de::DeserializeOwned + serde::Serialize + IntoValue,
{
    check::<T>(ruby, json!({"kind":"Empty"}), "Hash");
    let wire = if adjacent {
        json!({"kind":"Named", "data":{"label":"unchanged"}})
    } else {
        json!({"kind":"Named", "label":"unchanged"})
    };
    check::<T>(ruby, wire, "Hash");
}

fn main() {
    // SAFETY: This executable initializes Ruby once on its main thread and retains the guard.
    let ruby = unsafe { magnus::embed::init() };
    ruby.eval::<magnus::Value>("require 'json'; require 'sorbet-runtime'")
        .unwrap();
    let scenario = std::env::args().nth(1).unwrap();
    let reconstruct = scenario == "reconstruction";
    if scenario != "missing-classes" {
        let module = ruby.define_module("Fixture").unwrap();
        module.define_class("Payload", ruby.class_object()).unwrap();
    }
    if reconstruct || scenario == "fallback" {
        let source = std::fs::read_to_string(std::env::args().nth(2).unwrap()).unwrap();
        ruby.eval::<magnus::Value>(&source).unwrap();
    }
    if scenario == "fallback" {
        fallbacks::<PlainInternal>(&ruby, false);
        fallbacks::<BoxedInternal>(&ruby, false);
        fallbacks::<PlainAdjacent>(&ruby, true);
        fallbacks::<BoxedAdjacent>(&ruby, true);
        fallbacks::<RecordOnly>(&ruby, false);
        println!("10 runtime cases passed");
        return;
    }
    record::<PlainInternal>(
        &ruby,
        false,
        if reconstruct {
            "Fixture::PlainInternalRecord"
        } else {
            "Hash"
        },
        reconstruct,
    );
    record::<BoxedInternal>(
        &ruby,
        false,
        if reconstruct {
            "Fixture::BoxedInternalRecord"
        } else {
            "Hash"
        },
        reconstruct,
    );
    record::<PlainAdjacent>(
        &ruby,
        true,
        if reconstruct {
            "Fixture::PlainAdjacentRecord"
        } else {
            "Hash"
        },
        reconstruct,
    );
    record::<BoxedAdjacent>(
        &ruby,
        true,
        if reconstruct {
            "Fixture::BoxedAdjacentRecord"
        } else {
            "Hash"
        },
        reconstruct,
    );
    record::<AllTyped>(
        &ruby,
        false,
        if reconstruct { "Fixture::AllTypedRecord" } else { "Hash" },
        reconstruct,
    );
    println!("5 runtime cases passed");
}
