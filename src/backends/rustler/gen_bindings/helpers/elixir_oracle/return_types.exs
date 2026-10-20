[{SampleShelf, binary}] = Code.compile_file(hd(System.argv()))
{:ok, entries} = Code.Typespec.fetch_specs(binary)
specs = Map.new(entries, fn {{name, _arity} = key, [spec]} ->
  {key, Code.Typespec.spec_to_quoted(name, spec) |> Macro.to_string()}
end)
keys = [{:list_books, 0}, {:list_books, 1}, {:all_books, 0}]
Enum.each(keys, fn key ->
  spec = Map.fetch!(specs, key)
  unless String.contains?(spec, "[SampleShelf.Book.t()]") do
    raise "wrong compiled return typespec for #{inspect(key)}: #{spec}"
  end
end)
unless String.contains?(Map.fetch!(specs, {:list_books, 1}), "String.t() | nil") do
  raise "optional string input changed"
end
IO.puts("RETURN_SPECS=3 INPUT_CONTROLS=1")
