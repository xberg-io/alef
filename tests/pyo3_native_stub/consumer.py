from sample import _sample as native
from sample.options import ContentConfig


def native_defaults() -> native.ContentConfig:
    """Exercise methods that belong to the native class."""
    value = native.ContentConfig.default()
    value.validate()
    return value


def public_constructor() -> ContentConfig:
    """Construct the public dataclass through its own typed surface."""
    return ContentConfig(label="public")
