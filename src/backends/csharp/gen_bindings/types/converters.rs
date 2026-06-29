use super::super::csharp_file_header;

pub(crate) fn gen_byte_array_to_int_array_converter(namespace: &str) -> String {
    use crate::backends::csharp::template_env::render;

    let mut out = csharp_file_header();
    out.push_str("using System;\n");
    out.push_str("using System.Collections.Generic;\n");
    out.push_str("using System.Text.Json;\n");
    out.push_str("using System.Text.Json.Serialization;\n\n");

    out.push_str(&render("namespace_decl.jinja", minijinja::context! { namespace }));
    out.push('\n');

    out.push_str("/// <summary>\n");
    out.push_str("/// Converts byte arrays to and from JSON integer arrays.\n");
    out.push_str("/// </summary>\n");
    out.push_str("/// <remarks>\n");
    out.push_str("/// System.Text.Json serializes byte[] as base64 strings by default, but Rust's serde\n");
    out.push_str("/// for Vec&lt;u8&gt; expects JSON arrays of integers [72, 101, 108, ...].\n");
    out.push_str("/// Apply this converter to byte[] fields that are serialized to FFI with\n");
    out.push_str("/// [JsonConverter(typeof(ByteArrayJsonConverter))]. On read it accepts BOTH a JSON\n");
    out.push_str("/// number array and a base64 string, so payloads from either convention round-trip.\n");
    out.push_str("/// </remarks>\n");
    out.push_str("public sealed class ByteArrayJsonConverter : JsonConverter<byte[]?>\n");
    out.push_str("{\n");
    out.push_str("    /// <summary>\n");
    out.push_str("    /// Reads a byte array from either a JSON array of integers or a base64 string.\n");
    out.push_str("    /// </summary>\n");
    out.push_str("    public override byte[]? Read(\n");
    out.push_str("        ref Utf8JsonReader reader,\n");
    out.push_str("        Type typeToConvert,\n");
    out.push_str("        JsonSerializerOptions options)\n");
    out.push_str("    {\n");
    out.push_str("        if (reader.TokenType == JsonTokenType.Null)\n");
    out.push_str("        {\n");
    out.push_str("            return null;\n");
    out.push_str("        }\n\n");
    out.push_str("        if (reader.TokenType == JsonTokenType.String)\n");
    out.push_str("        {\n");
    out.push_str("            return reader.GetBytesFromBase64();\n");
    out.push_str("        }\n\n");
    out.push_str("        if (reader.TokenType != JsonTokenType.StartArray)\n");
    out.push_str("        {\n");
    out.push_str("            throw new JsonException($\"Expected JSON array or base64 string for byte[], got {reader.TokenType}\");\n");
    out.push_str("        }\n\n");
    out.push_str("        var bytes = new List<byte>();\n");
    out.push_str("        while (reader.Read())\n");
    out.push_str("        {\n");
    out.push_str("            if (reader.TokenType == JsonTokenType.EndArray)\n");
    out.push_str("            {\n");
    out.push_str("                break;\n");
    out.push_str("            }\n");
    out.push_str("            if (reader.TokenType == JsonTokenType.Number)\n");
    out.push_str("            {\n");
    out.push_str("                bytes.Add((byte)reader.GetInt32());\n");
    out.push_str("            }\n");
    out.push_str("            else\n");
    out.push_str("            {\n");
    out.push_str("                throw new JsonException($\"Unexpected token type: {reader.TokenType}\");\n");
    out.push_str("            }\n");
    out.push_str("        }\n\n");
    out.push_str("        return bytes.ToArray();\n");
    out.push_str("    }\n\n");
    out.push_str("    /// <summary>\n");
    out.push_str("    /// Writes a byte array as a JSON array of integers.\n");
    out.push_str("    /// </summary>\n");
    out.push_str("    public override void Write(\n");
    out.push_str("        Utf8JsonWriter writer,\n");
    out.push_str("        byte[]? value,\n");
    out.push_str("        JsonSerializerOptions options)\n");
    out.push_str("    {\n");
    out.push_str("        if (value is null)\n");
    out.push_str("        {\n");
    out.push_str("            writer.WriteNullValue();\n");
    out.push_str("            return;\n");
    out.push_str("        }\n\n");
    out.push_str("        writer.WriteStartArray();\n");
    out.push_str("        foreach (var b in value)\n");
    out.push_str("        {\n");
    out.push_str("            writer.WriteNumberValue(b);\n");
    out.push_str("        }\n");
    out.push_str("        writer.WriteEndArray();\n");
    out.push_str("    }\n");
    out.push_str("}\n");

    out
}

/// Generate `JsonLeniency.cs`: a helper that strips unknown properties from a JSON
/// object before deserialization, so payloads carrying extra fields still parse into
/// types that do not declare them.
pub(crate) fn gen_json_leniency(namespace: &str) -> String {
    use crate::backends::csharp::template_env::render;

    let mut out = csharp_file_header();
    out.push_str("using System.Collections.Generic;\n");
    out.push_str("using System.Text.Json;\n\n");

    out.push_str(&render("namespace_decl.jinja", minijinja::context! { namespace }));
    out.push('\n');

    out.push_str("/// <summary>\n");
    out.push_str("/// Utility for lenient JSON deserialization that ignores unknown properties.\n");
    out.push_str("/// </summary>\n");
    out.push_str("internal static class JsonLeniency\n");
    out.push_str("{\n");
    out.push_str("    /// <summary>\n");
    out.push_str("    /// Remove unknown properties from a JSON object before deserialization, so JSON\n");
    out.push_str("    /// carrying extra fields still parses into a type that does not declare them.\n");
    out.push_str("    /// </summary>\n");
    out.push_str("    /// <param name=\"json\">The JSON string to filter.</param>\n");
    out.push_str("    /// <param name=\"knownProperties\">Set of property names that are known/allowed.</param>\n");
    out.push_str("    /// <returns>A JSON string with unknown properties removed.</returns>\n");
    out.push_str("    public static string FilterUnknownProperties(string json, HashSet<string> knownProperties)\n");
    out.push_str("    {\n");
    out.push_str("        if (string.IsNullOrEmpty(json) || json.Trim() == \"{}\" || json.Trim() == \"[]\" || json.Trim() == \"null\")\n");
    out.push_str("        {\n");
    out.push_str("            return json;\n");
    out.push_str("        }\n\n");
    out.push_str("        try\n");
    out.push_str("        {\n");
    out.push_str("            using var document = JsonDocument.Parse(json);\n");
    out.push_str("            if (document.RootElement.ValueKind != JsonValueKind.Object)\n");
    out.push_str("            {\n");
    out.push_str("                return json;\n");
    out.push_str("            }\n\n");
    out.push_str("            using var stream = new System.IO.MemoryStream();\n");
    out.push_str("            using var writer = new Utf8JsonWriter(stream);\n\n");
    out.push_str("            writer.WriteStartObject();\n");
    out.push_str("            foreach (var property in document.RootElement.EnumerateObject())\n");
    out.push_str("            {\n");
    out.push_str("                if (knownProperties.Contains(property.Name))\n");
    out.push_str("                {\n");
    out.push_str("                    writer.WritePropertyName(property.Name);\n");
    out.push_str("                    property.Value.WriteTo(writer);\n");
    out.push_str("                }\n");
    out.push_str("            }\n");
    out.push_str("            writer.WriteEndObject();\n");
    out.push_str("            writer.Flush();\n\n");
    out.push_str("            return System.Text.Encoding.UTF8.GetString(stream.ToArray());\n");
    out.push_str("        }\n");
    out.push_str("        catch\n");
    out.push_str("        {\n");
    out.push_str("            // If filtering fails, return the original JSON and let deserialization handle it.\n");
    out.push_str("            return json;\n");
    out.push_str("        }\n");
    out.push_str("    }\n");
    out.push_str("}\n");

    out
}
