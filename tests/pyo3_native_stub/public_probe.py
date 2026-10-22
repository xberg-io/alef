# ruff: noqa: S101 ~keep Assertions belong to this generated-code regression fixture.
import sys
from types import ModuleType


def verify_public(native: ModuleType, package_root: str) -> None:
    """Check the public dataclass against the native class it shadows."""
    sys.path.insert(0, package_root)
    sys.modules["sample._sample"] = native
    import sample.options as public  # noqa: PLC0415 ~keep The module needs the native module registered first.

    value = native.ContentConfig.default()
    assert value.label == "native-default"
    assert value.validate() is None
    config = public.ContentConfig(label="public")
    assert config.label == "public"
    # A static helper has no `self` to convert, so it stays on the native class.
    assert not hasattr(public.ContentConfig, "default")
    # An instance method reaches the native class with the fields of the dataclass.
    assert config.validate() is None
    assert config.describe() == "config:public"
    assert config.describe() == native.ContentConfig(label="public").describe()


def verify_invalid_json(native: ModuleType) -> None:
    """Check that a JSON-typed field rejects a string that is not valid JSON, and names the field."""
    assert native.RequestConfig(extra_body='{"key": 1}', prediction='[1]') is not None
    assert native.RequestConfig(extra_body="{}") is not None
    for field, arguments in (
        ("extra_body", {"extra_body": "{not json"}),
        ("prediction", {"extra_body": "{}", "prediction": "{not json"}),
    ):
        try:
            native.RequestConfig(**arguments)
        except ValueError as error:
            assert f"invalid JSON in `{field}`" in str(error), str(error)
        else:
            raise AssertionError(f"RequestConfig accepted invalid JSON in {field}")
