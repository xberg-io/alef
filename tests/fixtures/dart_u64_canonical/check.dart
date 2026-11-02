import 'dart:async';
import 'dart:convert';
import 'dart:io';

import 'package:flutter_rust_bridge/flutter_rust_bridge_for_generated.dart';
import 'lib/src/wide_codec_bridge_generated/frb_generated.dart';
import 'lib/src/wide_codec_bridge_generated/lib.dart' as raw;

final maximum = (BigInt.one << 64) - BigInt.one;
const deadline = Duration(seconds: 5);

void require(bool condition, String message) {
  if (!condition) throw StateError(message);
}

Future<Map<String, Object>> rawCase(String mode) async {
  if (mode == 'signed') {
    final value = await raw.echoSigned(value: -1).timeout(deadline);
    require(value == -1, 'signed -1 changed');
    require(await raw.scalarCalls() == 1, 'signed core entry must be exactly one');
    return {'qualified_roundtrip': true, 'value': '$value', 'native_calls': 1};
  }
  final input = mode == 'raw-max' ? maximum : mode == 'raw-negative' ? -BigInt.one : maximum + BigInt.one;
  if (mode == 'raw-max') {
    final value = await raw.rawScalar(value: input).timeout(deadline);
    require(value is BigInt && value == maximum, 'maximum unsigned native roundtrip changed');
    require(await raw.scalarCalls() == 1, 'raw core entry must be exactly one');
    return {'qualified_roundtrip': true, 'value': '$value', 'native_calls': 1};
  }
  try {
    final value = await raw.rawScalar(value: input).timeout(deadline);
    final calls = await raw.scalarCalls().timeout(deadline);
    throw StateError('expected RangeError before native entry: value=$value calls=$calls');
  } on RangeError catch (error) {
    require('$error'.contains('u64 unsigned value'), 'wrong range rejection: $error');
    require(await raw.scalarCalls() == 0, 'invalid raw argument reached native core');
    return {'qualified_roundtrip': false, 'rejected': true, 'native_calls': 0};
  }
}

Future<Map<String, Object>> callbackCase(String mode) async {
  var callbacks = 0;
  final implementation = await raw.createWideCallbackDartImpl(value: (BigInt input) {
    callbacks++;
    require(input == maximum, 'callback input lost the full u64 range');
    if (mode == 'callback-body-error') throw StateError('callback-body-control');
    if (mode == 'callback-negative') return -BigInt.one;
    if (mode == 'callback-overflow') return maximum + BigInt.one;
    return maximum;
  }).timeout(deadline);
  await raw.registerWideCallback(impl: implementation).timeout(deadline);
  final value = await raw.invokeCallback(input: maximum).timeout(deadline);
  require(callbacks == 1, 'exactly one actual Dart callback must run');
  require(await raw.callbackStarted() == 1, 'exactly one native callback invocation must start');
  require(await raw.callbackFinished() == 1, 'native callback invocation must complete');
  require(value is BigInt, 'callback carrier is not BigInt');
  if (mode == 'callback-max') {
    require(value == maximum, 'maximum callback native roundtrip changed');
    return {'qualified_roundtrip': true, 'value': '$value', 'callbacks': callbacks};
  }
  // The emitted synchronous infallible trait warns and substitutes Default on callback failure.
  // Rust's parent harness also requires that warning and the original rejection/error cause.
  require(value == BigInt.zero, 'callback error fallback must not wrap an invalid value: $value');
  return {
    'qualified_roundtrip': false,
    'error_fallback': 'warned-default-zero',
    'rejected': mode != 'callback-body-error',
    'value': '$value',
    'callbacks': callbacks,
  };
}

void main(List<String> args) {
  final uncaught = <Object>[];
  Timer(const Duration(seconds: 20), () {
    stderr.writeln('canonical fixture exceeded process deadline');
    exit(2);
  });
  runZonedGuarded(() async {
    try {
      require(args.length == 2, 'expected native library and case mode');
      final mode = args[1];
      require(const {
        'raw-max', 'raw-negative', 'raw-overflow', 'signed',
        'callback-max', 'callback-negative', 'callback-overflow', 'callback-body-error',
      }.contains(mode), 'unknown case mode');
      await RustLib.init(externalLibrary: ExternalLibrary.open(args[0])).timeout(deadline);
      require(await raw.installWarningCapture(), 'fixture warning subscriber must install');
      await raw.resetCounts().timeout(deadline);
      final receipt = await (mode.startsWith('callback-') ? callbackCase(mode) : rawCase(mode));
      await Future<void>.delayed(Duration.zero);
      require(uncaught.isEmpty, 'uncaught async errors are not successful range transport: $uncaught');
      stdout.writeln('canonical_receipt=${jsonEncode({'mode': mode, ...receipt, 'uncaught_async': 0})}');
      await stdout.flush();
      exit(0);
    } catch (error, stack) {
      stderr.writeln('canonical case failed: $error\n$stack\nuncaught_async=$uncaught');
      await stderr.flush();
      exit(1);
    }
  }, (Object error, StackTrace stack) {
    uncaught.add(error);
    stderr.writeln('uncaught callback encoder error: $error\n$stack');
  });
}
