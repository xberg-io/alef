import 'dart:typed_data';
import 'package:flutter_rust_bridge/flutter_rust_bridge_for_generated.dart';
import 'lib/bridge/frb_generated.dart';
import 'lib/bridge/lib.dart';

const deadline = Duration(seconds: 30);

Future<void> main(List<String> args) async {
  await RustLib.init(externalLibrary: ExternalLibrary.open(args.single)).timeout(deadline);
  final client = await createClient();
  var checked = 0;
  for (final mode in [1, 2, 3, 4, 0]) {
    final chunks = <String>[];
    Object? failure;
    try {
      await for (final chunk in client.records(mode: mode).timeout(deadline)) {
        chunks.add(chunk.text);
      }
    } catch (error) {
      failure = error;
    }
    if (mode == 0) {
      if (failure != null || chunks.join(',') != 'first,second') {
        throw StateError('normal stream failed: $failure $chunks');
      }
    } else if (mode == 1) {
      if (failure is! StreamError_Authentication || failure.status != 401 ||
        failure.message != 'bad key' || chunks.isNotEmpty) {
        throw StateError('expected typed Authentication: $failure $chunks');
      }
    } else if (mode == 2) {
      if (failure is! StreamError_RateLimited || failure.status != 429 ||
        failure.retryMs.toString() != '7000' || failure.message != 'slow down' ||
        chunks.join(',') != 'first,second') {
        throw StateError('expected typed RateLimited after chunks: $failure $chunks');
      }
    } else if (mode == 3) {
      if (failure is! StreamError_ContentPolicy || failure.status != 403 ||
        failure.message != 'blocked' || chunks.isNotEmpty) {
        throw StateError('expected typed ContentPolicy: $failure $chunks');
      }
    } else if (failure is! StateError || !failure.message.contains('future failure')) {
      throw StateError('unknown variant lost its native failure: $failure');
    }
    checked++;
  }
  final cancelled = await client.records(mode: 0).take(1).toList().timeout(deadline);
  if (cancelled.length != 1 || cancelled.single.text != 'first') {
    throw StateError('early stream cancellation lost the first item');
  }
  checked++;
  final scalar = await client.scalarRecords().first.timeout(deadline);
  if (scalar != BigInt.from(42)) throw StateError('u64 changed its Dart type: $scalar');
  checked++;
  final optional = await client.optionalRecords().toList().timeout(deadline);
  if (optional.length != 2 || optional.first != BigInt.from(42) || optional.last != null) {
    throw StateError('optional u64 changed: $optional');
  }
  checked++;
  final vector = await client.vectorRecords().single.timeout(deadline);
  if (vector is! Uint32List || vector.join(',') != '1,2') throw StateError('u32 vector changed: $vector');
  checked++;
  final floats = await client.floatRecords().single.timeout(deadline);
  if (floats is! Float32List || floats.join(',') != '1.5,2.5') throw StateError('f32 vector changed: $floats');
  checked++;
  final units = await client.unitRecords().toList().timeout(deadline);
  if (units.length != 1) throw StateError('unit stream changed');
  checked++;
  final unsignedBoundaries = await client.scalarRecords().toList().timeout(deadline);
  final maximum = (BigInt.one << 64) - BigInt.one;
  if (unsignedBoundaries.length != 2 || unsignedBoundaries.first != BigInt.from(42) ||
    unsignedBoundaries.last is! BigInt || unsignedBoundaries.last != maximum) {
    throw StateError('u64 MAX lost full range in stream: $unsignedBoundaries');
  }
  checked++;
  if (checked != 12) throw StateError('expected twelve stream cases: $checked');
  print('stream_cases=$checked');
}
