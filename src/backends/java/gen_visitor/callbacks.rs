//! Callback specifications for the metadata-driven Java visitor bridge.

pub struct CallbackSpec {
    /// Field name in the generated native visitor callback table.
    pub c_field: String,
    /// Java interface method name.
    pub java_method: String,
    /// Javadoc line.
    pub doc: String,
    /// Extra parameters beyond the configured context type in the Java interface.
    pub extra: Vec<ExtraParam>,
}

pub struct ExtraParam {
    /// Java parameter name in the interface.
    pub java_name: String,
    /// Java type in the interface method signature.
    pub java_type: String,
    /// Panama `ValueLayout` constants for each C-level argument that maps to this Java param.
    pub c_layouts: Vec<String>,
    /// Java expression to build the interface-typed value from the raw C parameters.
    pub decode: String,
}
