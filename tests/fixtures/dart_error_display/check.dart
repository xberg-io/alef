import 'dart:io';
import 'package:flutter_rust_bridge/flutter_rust_bridge_for_generated.dart';
import 'lib/src/display_sample_bridge_generated/frb_generated.dart';
import 'lib/src/display_sample_bridge_generated/lib.dart';

Future<void> main(List<String> args) async {
  if (args.first == 'constructors-only') {
    final error = SampleError.missing(message: 'missing payload', status: 404);
    if (error.toString() != 'not_found: missing payload') {
      throw StateError('expected display 0: not_found: missing payload, actual: $error');
    }
    return;
  }
  await RustLib.init(externalLibrary: ExternalLibrary.open(args.first));
  final expected = [
    'not_found: missing payload',
    'policy_rejected: https://example.test/ - denied',
    'numbers: 7/true',
    'escaped { raw \$ {value} } \$quote " slash \\ newline\n',
    'unavailable',
    'keyword: required raw',
  ];
  if (args.length > 1) expected[0] = 'deliberately wrong display';
  var checked = 0;
  for (var mode = 0; mode < expected.length; mode++) {
    Object? failure;
    try {
      await failCase(mode: mode);
    } catch (error) {
      failure = error;
    }
    if (failure.toString() != expected[mode]) {
      throw StateError('expected display $mode: ${expected[mode]}, actual: $failure');
    }
    switch (failure) {
      case SampleError_Missing(:final message, :final status):
        if (message != 'missing payload' || status != 404) throw StateError('changed raw payload');
      case SampleError_Policy(:final url, :final reason):
        if (url != 'https://example.test/' || reason != 'denied') throw StateError('changed policy payload');
      case SampleError_Numbers(:final count, :final enabled):
        if (count != 7 || !enabled) throw StateError('changed numeric payload');
      case SampleError_Escaped(:final message):
        if (message != r'raw $ {value}') throw StateError('changed escaped payload');
      case SampleError_Unit():
        break;
      case SampleError_Keyword(:final required_):
        if (required_ != 'required raw') throw StateError('changed keyword payload');
      default:
        throw StateError('typed variant matching was lost');
    }
    checked++;
  }
  final constructed = SampleError.missing(message: 'caller raw', status: 401);
  if (constructed is! SampleError_Missing || constructed.message != 'caller raw' || constructed.toString() != 'not_found: caller raw') {
    throw StateError('public constructor or payload changed');
  }
  checked++;
  final debug = SampleError.debug(message: 'debug raw');
  if (!debug.toString().contains('SampleError.debug(')) throw StateError('unsupported format changed');
  checked++;
  if (SimpleError.only.toString() != 'SimpleError.only') throw StateError('plain unit enum changed');
  checked++;
  if (checked != 9) throw StateError('expected nine Dart runtime cases, got $checked');
  stdout.writeln('error_display_cases=$checked');
}
