require ARGV.fetch(0)

checks = 0
check = lambda do |label, expected, actual|
  abort("CHECK_FAILED #{label}: expected #{expected.inspect}, got #{actual.inspect}") unless actual == expected
  checks += 1
end

capture = lambda do |label, call, family|
  begin
    RubyFamilyNative.public_send(call)
  rescue family => error
    error
  rescue StandardError => error
    abort("CHECK_FAILED #{label}: #{error.class} bypassed #{family}")
  else
    abort("CHECK_FAILED #{label}: native converter did not raise")
  end
end

m = RubyFamilyNative
m.send(:remove_const, :RootOnlyError) if ARGV.include?("--poison-root-only")
shared = m::SharedError
manual = shared.new("manual")
check.call("shared_name", "RubyFamilyNative::SharedError", shared.name)
check.call("old_superclass", m::GraphQLError, shared.superclass)
check.call("schema_superclass", m::RootError, m::SchemaError.superclass)
check.call("old_message_constructor", "manual", manual.message)
check.call("first_nominal_family", true, manual.is_a?(m::GraphQLError))
check.call("second_module_membership", true, manual.is_a?(m::SchemaError::Family))
check.call("ancestor_module_membership", true, manual.is_a?(m::RootError::Family))
check.call("no_second_nominal_inheritance", false, manual.is_a?(m::SchemaError))
check.call("first_family_module", true, shared.ancestors.include?(m::GraphQLError::Family))
check.call("second_family_module", true, shared.ancestors.include?(m::SchemaError::Family))
check.call("ancestor_family_module", true, shared.ancestors.include?(m::RootError::Family))
check.call("graph_only_is_not_schema", false, m::SchemaError === m::GraphOnlyError.new("only"))
check.call("schema_only_is_not_graph", false, m::GraphQLError === m::SchemaOnlyError.new("only"))
check.call("variant_rescue_is_not_family_rescue", false, m::GraphOnlyError === manual)
check.call("graph_variant_retains_itself", true, m::GraphOnlyError === m::GraphOnlyError.new("only"))
check.call("schema_variant_retains_itself", true, m::SchemaOnlyError === m::SchemaOnlyError.new("only"))

graph = capture.call("first_graph_family", :graph_shared, m::GraphQLError)
schema = capture.call("first_schema_family", :schema_shared, m::SchemaError)
check.call("graph_converter_class", shared, graph.class)
check.call("schema_converter_class", shared, schema.class)
check.call("graph_converter_message", "graphql", graph.message)
check.call("schema_converter_message", "schema", schema.message)
check.call("graph_introspection", 403, graph.status_code)
check.call("schema_introspection", 429, schema.status_code)
check.call("other_family_value_absent", nil, graph.error_type)
check.call("own_family_value_present", "schema-kind", schema.error_type)
if ARGV.include?("--poison-family")
  class << RubyFamilyNative::SchemaError
    def ===(other)
      super
    end
  end
end

check.call(
  "secondary_graph_schema",
  shared,
  capture.call("secondary_graph_schema", :graph_shared, m::SchemaError).class
)
check.call(
  "secondary_schema_graph",
  shared,
  capture.call("secondary_schema_graph", :schema_shared, m::GraphQLError).class
)
check.call("ancestor_graph", shared, capture.call("ancestor_graph", :graph_shared, m::RootError).class)
check.call("ancestor_schema", shared, capture.call("ancestor_schema", :schema_shared, m::RootError).class)
graph_only = capture.call("graph_only", :graph_only, m::GraphQLError)
schema_only = capture.call("schema_only", :schema_only, m::SchemaError)
check.call("graph_only_converter", m::GraphOnlyError, graph_only.class)
check.call("schema_only_converter", m::SchemaOnlyError, schema_only.class)
check.call("graph_only_converter_nonmember", false, m::SchemaError === graph_only)
check.call("schema_only_converter_nonmember", false, m::GraphQLError === schema_only)
check.call("root_legacy_superclass", StandardError, m::RootError.superclass)
check.call("root_only_superclass", m::RootError, m::RootOnlyError.superclass)
check.call("root_only_constructor", "root", m::RootOnlyError.new("root").message)
check.call("root_only_family", true, m::RootError === m::RootOnlyError.new("root"))
check.call("base_constructor", "base", m::GraphQLError.new("base").message)
check.call("ancestor_rejects_unrelated", false, m::RootError === RuntimeError.new("unrelated"))
check.call("schema_rejects_unrelated", false, m::SchemaError === RuntimeError.new("unrelated"))
abort("CHECK_FAILED census: #{checks}/39") unless checks == 39
puts("PASS: 39 checks; 2 converters; 8 negative oracles")
