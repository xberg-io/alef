mod items;
mod metadata;
mod ordered_serde;
mod service;
mod surface;
mod type_ref;

pub(crate) use items::ADAPTER_HANDLED_REASON_PREFIX;
pub use items::{
    EnumDef, EnumVariant, ErrorDef, ErrorVariant, FieldDef, FunctionDef, MethodDef, ParamDef, ReceiverKind, TypeDef,
};
pub use metadata::{
    CoreWrapper, DefaultValue, DeprecationInfo, ErrorTaxonomy, SerdeContainerConversion, VersionAnnotation,
};
pub use service::{
    EntrypointDef, EntrypointKind, HandlerContractDef, HandlerShape, ParameterConstraint, RegistrationDef,
    RegistrationVariant, RegistrationVariantLanguageOverride, RegistrationVariantOverride, RegistrationVariantStyle,
    ResolvedVariant, ServiceDef, WrapperConstructorArg, WrapperConstructorCall, WrapperOption,
};
pub use surface::{ApiSurface, UnsupportedPublicItem, cfg_feature_satisfied};
pub use type_ref::{PrimitiveType, TypeRef};
