import 'dart:convert';
import 'dart:io';
import 'package:flutter_rust_bridge/flutter_rust_bridge.dart' as frb;
import 'package:flutter_rust_bridge/flutter_rust_bridge_for_generated.dart' show ExternalLibrary;
import 'lib/src/wide_sample_bridge_generated/frb_generated.dart';
import 'lib/src/wide_sample.dart';

final boundaries = <BigInt>[
  BigInt.zero,
  (BigInt.one << 53) + BigInt.one,
  (BigInt.one << 63) - BigInt.one,
  BigInt.one << 63,
  (BigInt.one << 64) - BigInt.one,
];

Map<String, dynamic> recordJson(BigInt value) => {
  'value': 'UNSIGNED_VALUE', 'optional': 'UNSIGNED_VALUE', 'values': ['UNSIGNED_VALUE'],
  'named': {'item': 'UNSIGNED_VALUE'}, 'wrapped': 'UNSIGNED_VALUE',
  'byte': 255, 'short': 65535, 'count': 4294967295, 'signed': -1,
};

String encodedRecord(BigInt value) => jsonEncode(recordJson(value)).replaceAll('"UNSIGNED_VALUE"', value.toString());

void sameUnsigned(dynamic actual, BigInt expected, String context) {
  if (actual is! BigInt || actual != expected) {
    throw StateError('unsigned boundary mismatch $context expected=$expected actual=$actual type=${actual.runtimeType}');
  }
}

void sameRecord(dynamic record, BigInt value) {
  sameUnsigned(record.value, value, 'record scalar');
  sameUnsigned(record.optional, value, 'record optional');
  sameUnsigned(record.values.single, value, 'record vector');
  sameUnsigned(record.named['item'], value, 'record map');
  sameUnsigned(record.wrapped, value, 'record newtype');
  if (record.byte != 255 || record.short != 65535 || record.count != 4294967295 || record.signed != -1) {
    throw StateError('narrow or signed control changed');
  }
}

Future<void> rejects(Future<dynamic> Function() call, String context) async {
  Object? failure;
  try { await call(); } catch (error) { failure = error; }
  if (failure == null) throw StateError('invalid unsigned value was accepted: $context');
  if (!RegExp(r'u64|range|unsigned|unknown field|missing field|invalid type').hasMatch(failure.toString())) {
    throw StateError('wrong-reason rejection $context: $failure');
  }
}

WideRecord changeRecord(dynamic record, String field, BigInt value) => WideRecord(
  value: field == 'value' ? value : record.value,
  optional: field == 'optional' ? value : record.optional,
  values: record.values,
  named: field == 'named' ? {'item': value} : record.named,
  wrapped: field == 'wrapped' ? value : record.wrapped,
  byte: record.byte, short: record.short, count: record.count, signed: record.signed,
);

WideNested nestedRecord(dynamic record, {String? placement, dynamic changed}) => WideNested(
  record: placement == 'record' ? changed : record,
  optional: placement == 'optional' ? changed : record,
  records: [placement == 'records' ? changed : record],
  named: {'item': placement == 'named' ? changed : record},
);

void sameNested(dynamic nested, BigInt value) {
  sameRecord(nested.record, value);
  sameRecord(nested.optional, value);
  sameRecord(nested.records.single, value);
  sameRecord(nested.named['item'], value);
}

Future<void> expectEntries(dynamic probe, int expected, String context) async {
  final actual = await WideSampleBridge.probeCalls(probe);
  if (actual.toString() != expected.toString()) {
    throw StateError('native entry mismatch $context expected=$expected actual=$actual');
  }
}

Future<void> main(List<String> args) async {
  await RustLib.init(externalLibrary: ExternalLibrary.open(args.first));
  var checked = 0;
  for (var index = 0; index < boundaries.length; index++) {
    final value = boundaries[index];
    final oracle = args.length > 1 && index == 0 ? value + BigInt.one : value;
    sameUnsigned(await WideSampleBridge.scalarForCase(index), oracle, 'core return'); checked++;
    sameUnsigned(await WideSampleBridge.echoScalar(value), value, 'scalar roundtrip'); checked++;
    sameUnsigned(await WideSampleBridge.echoOptional(value), value, 'optional roundtrip'); checked++;
    final vector = await WideSampleBridge.echoVector(frb.Uint64List.fromList([value]));
    if (vector is! frb.Uint64List) throw StateError('unsigned list lost FRB BigInt representation');
    sameUnsigned(vector.single, value, 'vector roundtrip'); checked++;
    final named = await WideSampleBridge.echoMap({'item': value});
    sameUnsigned(named['item'], value, 'map roundtrip'); checked++;
    sameUnsigned(await WideSampleBridge.echoNewtype(value), value, 'newtype roundtrip'); checked++;
    final record = await createWideRecordFromJson(json: encodedRecord(value));
    sameRecord(record, value); checked++;
    sameRecord(await WideSampleBridge.echoRecord(record), value); checked++;
  }
  if (await WideSampleBridge.echoOptional(null) != null) throw StateError('optional null changed'); checked++;
  if ((await WideSampleBridge.echoSigned(-1)).toString() != '-1') throw StateError('signed -1 changed'); checked++;
  if ((await WideSampleBridge.echoU32(4294967295)).toString() != '4294967295') throw StateError('u32 changed'); checked++;
  for (final invalid in [-BigInt.one, BigInt.one << 64]) {
    await rejects(() => WideSampleBridge.echoScalar(invalid), 'scalar $invalid'); checked++;
    await rejects(() => WideSampleBridge.echoOptional(invalid), 'optional $invalid'); checked++;
    await rejects(() => WideSampleBridge.echoMap({'item': invalid}), 'map $invalid'); checked++;
    await rejects(() => createWideRecordFromJson(json: encodedRecord(invalid)), 'JSON $invalid'); checked++;
  }
  final missing = (jsonDecode(encodedRecord(BigInt.zero)) as Map<String, dynamic>)..remove('value');
  await rejects(() => createWideRecordFromJson(json: jsonEncode(missing)), 'missing required'); checked++;
  final unknown = encodedRecord(BigInt.zero).replaceFirst('{', '{"unexpected":true,');
  await rejects(() => createWideRecordFromJson(json: unknown), 'unknown field'); checked++;
  final explicitNull = encodedRecord(BigInt.zero).replaceFirst('"value":0', '"value":null');
  await rejects(() => createWideRecordFromJson(json: explicitNull), 'null non-optional'); checked++;
  if (checked != 54) throw StateError('original54 case inventory changed: $checked');
  final addedNames = <String>[];
  Future<void> added(String name, Future<void> Function() body) async {
    if (addedNames.contains(name)) throw StateError('duplicate case $name');
    await body();
    addedNames.add(name);
    checked++;
  }
  final probe = await WideSampleBridge.createProbe();
  final zero = await createWideRecordFromJson(json: encodedRecord(BigInt.zero));
  for (final value in boundaries) {
    final record = await createWideRecordFromJson(json: encodedRecord(value));
    await added('nested-valid-$value', () async {
      await WideSampleBridge.probeReset(probe);
      sameNested(await WideSampleBridge.checkedNested(probe, nestedRecord(record)), value);
      await expectEntries(probe, 1, 'nested valid');
    });
    await added('newtype-counted-valid-$value', () async {
      await WideSampleBridge.probeReset(probe);
      sameUnsigned(await WideSampleBridge.checkedNewtype(probe, value), value, 'counted newtype');
      await expectEntries(probe, 1, 'newtype valid');
    });
    await added('record-counted-valid-$value', () async {
      await WideSampleBridge.probeReset(probe);
      sameRecord(await WideSampleBridge.checkedRecord(probe, record), value);
      await expectEntries(probe, 1, 'record valid');
    });
  }
  for (final invalid in [-BigInt.one, BigInt.one << 64]) {
    await added('newtype-direct-invalid-$invalid', () => rejects(
      () => WideSampleBridge.echoNewtype(invalid), 'direct newtype $invalid'));
    await added('newtype-counted-invalid-$invalid', () async {
      await WideSampleBridge.probeReset(probe);
      await rejects(() => WideSampleBridge.checkedNewtype(probe, invalid), 'counted newtype $invalid');
      await expectEntries(probe, 0, 'invalid newtype must not enter Rust core');
    });
    for (final field in ['value', 'optional', 'named', 'wrapped']) {
      await added('record-direct-invalid-$field-$invalid', () => rejects(
        () => WideSampleBridge.echoRecord(changeRecord(zero, field, invalid)), 'direct record $field $invalid'));
      await added('record-counted-invalid-$field-$invalid', () async {
        await WideSampleBridge.probeReset(probe);
        await rejects(() => WideSampleBridge.checkedRecord(probe, changeRecord(zero, field, invalid)), 'counted record $field $invalid');
        await expectEntries(probe, 0, 'invalid record must not enter Rust core');
      });
      for (final placement in ['record', 'optional', 'records', 'named']) {
        await added('nested-invalid-$placement-$field-$invalid', () async {
          await WideSampleBridge.probeReset(probe);
          await rejects(() => WideSampleBridge.checkedNested(probe,
            nestedRecord(zero, placement: placement, changed: changeRecord(zero, field, invalid))),
          'nested $placement $field $invalid');
          await expectEntries(probe, 0, 'invalid nested leaf must not enter Rust core');
        });
      }
    }
  }
  if (addedNames.length != 67 || checked != 121) throw StateError('expected121 cases, got $checked');
  stdout.writeln('u64_added_case_names=${jsonEncode(addedNames)}');
  stdout.writeln('u64_cases=$checked');
}
