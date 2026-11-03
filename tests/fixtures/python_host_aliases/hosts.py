class Legacy:
    """Implement only the original host callback protocol."""

    def name(self) -> str:
        """Return the plugin identity used in error context."""
        return "legacy"

    def count(self, text: str) -> int:
        """Count input or raise a recognizable callback error."""
        if text == "raise":
            raise RuntimeError("host-count-sentinel")
        return len(text)

    def alternate(self, text: str) -> int:
        """Provide a distinct valid alias target."""
        return len(text) + 10


class Override(Legacy):
    """Override the new optional host method explicitly."""

    def checked_count(self, _text: str) -> int:
        """Return a value distinguishable from the fallback."""
        return 97


class DescriptorFailure(Legacy):
    """Raise while resolving the optional method descriptor."""

    @property
    def checked_count(self) -> int:
        """Refuse lookup before a callback can run."""
        raise RuntimeError("descriptor-sentinel")


class AttributeDescriptorFailure(Legacy):
    """Raise AttributeError from an existing descriptor."""

    @property
    def checked_count(self) -> int:
        """Distinguish a failing descriptor from an absent method."""
        raise AttributeError("attribute-descriptor-sentinel")


class Missing:
    """Omit the required legacy host method."""
