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
