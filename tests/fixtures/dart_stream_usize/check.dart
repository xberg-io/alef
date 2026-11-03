import 'package:flutter_rust_bridge/flutter_rust_bridge_for_generated.dart' as frb;
import 'lib/bridge/frb_generated.dart';
import 'lib/bridge/lib.dart';

const deadline = Duration(seconds: 30);

Future<void> main(List<String> args) async {
  await RustLib.init(externalLibrary: frb.ExternalLibrary.open(args.single)).timeout(deadline);
  final client = await createClient();
  var checked = 0;
  final int scalar = await client.usizeScalar().single.timeout(deadline);
  if (scalar != 42) throw StateError('usize scalar lost its int representation: $scalar');
  checked++;
  final UsizeRecord record = await client.usizeRecord().single.timeout(deadline);
  final int count = record.count;
  if (count != 42) throw StateError('named usize record changed: $count');
  checked++;
  final List<int?> optional = await client.usizeOptional().toList().timeout(deadline);
  if (optional.length != 2 || optional.first != 42 || optional.last != null) {
    throw StateError('optional usize changed: $optional');
  }
  checked++;
  final frb.Int64List vector = await client.usizeVector().single.timeout(deadline);
  if (vector.join(',') != '0,42,1099511627776') throw StateError('usize vector changed: $vector');
  checked++;
  final List<int?> nested = await client.usizeNested().single.timeout(deadline);
  if (nested.length != 2 || nested.first != 42 || nested.last != null) {
    throw StateError('nested usize changed: $nested');
  }
  checked++;
  final maximum = (BigInt.one << 64) - BigInt.one;
  final Map<int, BigInt> mapped = await client.usizeMap().single.timeout(deadline);
  if (mapped.length != 1 || mapped[7] != maximum) throw StateError('usize map changed: $mapped');
  checked++;
  final BigInt unsigned = await client.u64Scalar().single.timeout(deadline);
  if (unsigned != maximum) throw StateError('u64 scalar lost full range: $unsigned');
  checked++;
  final frb.Uint64List unsignedVector = await client.u64Vector().single.timeout(deadline);
  if (unsignedVector.length != 1 || unsignedVector.single != maximum) {
    throw StateError('u64 vector lost full range: $unsignedVector');
  }
  checked++;
  if (checked != 8) throw StateError('expected eight usize/u64 stream cases: $checked');
  print('usize_stream_cases=$checked');
}
