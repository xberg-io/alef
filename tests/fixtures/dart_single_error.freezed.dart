// GENERATED CODE - DO NOT MODIFY BY HAND
// coverage:ignore-file
// ignore_for_file: type=lint, type=warning, deprecated_member_use, deprecated_member_use_from_same_package
// ignore_for_file: unused_element, deprecated_member_use, deprecated_member_use_from_same_package, use_function_type_syntax_for_parameters, unnecessary_const, avoid_init_to_null, invalid_override_different_default_values_named, prefer_expression_function_bodies, annotate_overrides, invalid_annotation_target, unnecessary_question_mark

part of 'lib.dart';

// **************************************************************************
// FreezedGenerator
// **************************************************************************

// GENERATED CODE - DO NOT MODIFY BY HAND
// dart format off
T _$identity<T>(T value) => value;
/// @nodoc
mixin _$AlefBridgeErrorWire {





  @override
  bool operator ==(Object other) {
    return identical(this, other) || (other.runtimeType == runtimeType&&other is AlefBridgeErrorWire);
  }


  @override
  int get hashCode => runtimeType.hashCode;

  @override
  String toString() {
    return 'AlefBridgeErrorWire()';
  }


}

/// @nodoc
class $AlefBridgeErrorWireCopyWith<$Res>  {
  $AlefBridgeErrorWireCopyWith(AlefBridgeErrorWire _, $Res Function(AlefBridgeErrorWire) __);
}


/// Adds pattern-matching-related methods to [AlefBridgeErrorWire].
extension AlefBridgeErrorWirePatterns on AlefBridgeErrorWire {
  /// A variant of `map` that fallback to returning `orElse`.
  ///
  /// It is equivalent to doing:
  /// ```dart
  /// switch (sealedClass) {
  ///   case final Subclass value:
  ///     return ...;
  ///   case _:
  ///     return orElse();
  /// }
  /// ```

  @optionalTypeArgs TResult maybeMap<TResult extends Object?>({TResult Function( AlefBridgeErrorWire_RequestError value)?  requestError,TResult Function( AlefBridgeErrorWire_InvalidInput value)?  invalidInput,required TResult orElse(),}){
    final _that = this;
    switch (_that) {
      case AlefBridgeErrorWire_RequestError() when requestError != null:
      return requestError(_that);case AlefBridgeErrorWire_InvalidInput() when invalidInput != null:
      return invalidInput(_that);case _:
      return orElse();

    }
  }
  /// A `switch`-like method, using callbacks.
  ///
  /// Callbacks receives the raw object, upcasted.
  /// It is equivalent to doing:
  /// ```dart
  /// switch (sealedClass) {
  ///   case final Subclass value:
  ///     return ...;
  ///   case final Subclass2 value:
  ///     return ...;
  /// }
  /// ```

  @optionalTypeArgs TResult map<TResult extends Object?>({required TResult Function( AlefBridgeErrorWire_RequestError value)  requestError,required TResult Function( AlefBridgeErrorWire_InvalidInput value)  invalidInput,}){
    final _that = this;
    switch (_that) {
      case AlefBridgeErrorWire_RequestError():
      return requestError(_that);case AlefBridgeErrorWire_InvalidInput():
      return invalidInput(_that);}
  }
  /// A variant of `map` that fallback to returning `null`.
  ///
  /// It is equivalent to doing:
  /// ```dart
  /// switch (sealedClass) {
  ///   case final Subclass value:
  ///     return ...;
  ///   case _:
  ///     return null;
  /// }
  /// ```

  @optionalTypeArgs TResult? mapOrNull<TResult extends Object?>({TResult? Function( AlefBridgeErrorWire_RequestError value)?  requestError,TResult? Function( AlefBridgeErrorWire_InvalidInput value)?  invalidInput,}){
    final _that = this;
    switch (_that) {
      case AlefBridgeErrorWire_RequestError() when requestError != null:
      return requestError(_that);case AlefBridgeErrorWire_InvalidInput() when invalidInput != null:
      return invalidInput(_that);case _:
      return null;

    }
  }
  /// A variant of `when` that fallback to an `orElse` callback.
  ///
  /// It is equivalent to doing:
  /// ```dart
  /// switch (sealedClass) {
  ///   case Subclass(:final field):
  ///     return ...;
  ///   case _:
  ///     return orElse();
  /// }
  /// ```

  @optionalTypeArgs TResult maybeWhen<TResult extends Object?>({TResult Function( RequestError error)?  requestError,TResult Function( String message)?  invalidInput,required TResult orElse(),}) {final _that = this;
    switch (_that) {
      case AlefBridgeErrorWire_RequestError() when requestError != null:
      return requestError(_that.error);case AlefBridgeErrorWire_InvalidInput() when invalidInput != null:
      return invalidInput(_that.message);case _:
      return orElse();

    }
  }
  /// A `switch`-like method, using callbacks.
  ///
  /// As opposed to `map`, this offers destructuring.
  /// It is equivalent to doing:
  /// ```dart
  /// switch (sealedClass) {
  ///   case Subclass(:final field):
  ///     return ...;
  ///   case Subclass2(:final field2):
  ///     return ...;
  /// }
  /// ```

  @optionalTypeArgs TResult when<TResult extends Object?>({required TResult Function( RequestError error)  requestError,required TResult Function( String message)  invalidInput,}) {final _that = this;
    switch (_that) {
      case AlefBridgeErrorWire_RequestError():
      return requestError(_that.error);case AlefBridgeErrorWire_InvalidInput():
      return invalidInput(_that.message);}
  }
  /// A variant of `when` that fallback to returning `null`
  ///
  /// It is equivalent to doing:
  /// ```dart
  /// switch (sealedClass) {
  ///   case Subclass(:final field):
  ///     return ...;
  ///   case _:
  ///     return null;
  /// }
  /// ```

  @optionalTypeArgs TResult? whenOrNull<TResult extends Object?>({TResult? Function( RequestError error)?  requestError,TResult? Function( String message)?  invalidInput,}) {final _that = this;
    switch (_that) {
      case AlefBridgeErrorWire_RequestError() when requestError != null:
      return requestError(_that.error);case AlefBridgeErrorWire_InvalidInput() when invalidInput != null:
      return invalidInput(_that.message);case _:
      return null;

    }
  }

}

/// @nodoc


class AlefBridgeErrorWire_RequestError extends AlefBridgeErrorWire {
  const AlefBridgeErrorWire_RequestError({required this.error}): super._();


  final  RequestError error;

  /// Create a copy of AlefBridgeErrorWire
  /// with the given fields replaced by the non-null parameter values.
  @JsonKey(includeFromJson: false, includeToJson: false)
  @pragma('vm:prefer-inline')
  $AlefBridgeErrorWire_RequestErrorCopyWith<AlefBridgeErrorWire_RequestError> get copyWith => _$AlefBridgeErrorWire_RequestErrorCopyWithImpl<AlefBridgeErrorWire_RequestError>(this, _$identity);



  @override
  bool operator ==(Object other) {
    return identical(this, other) || (other.runtimeType == runtimeType&&other is AlefBridgeErrorWire_RequestError&&(identical(other.error, error) || other.error == error));
  }


  @override
  int get hashCode {
    return Object.hash(runtimeType,error);
  }

  @override
  String toString() {
    return 'AlefBridgeErrorWire.requestError(error: $error)';
  }


}

/// @nodoc
abstract mixin class $AlefBridgeErrorWire_RequestErrorCopyWith<$Res> implements $AlefBridgeErrorWireCopyWith<$Res> {
  factory $AlefBridgeErrorWire_RequestErrorCopyWith(AlefBridgeErrorWire_RequestError value, $Res Function(AlefBridgeErrorWire_RequestError) _then) = _$AlefBridgeErrorWire_RequestErrorCopyWithImpl;
  @useResult
  $Res call({
    RequestError error
  });


  $RequestErrorCopyWith<$Res> get error;

}
/// @nodoc
class _$AlefBridgeErrorWire_RequestErrorCopyWithImpl<$Res>
implements $AlefBridgeErrorWire_RequestErrorCopyWith<$Res> {
  _$AlefBridgeErrorWire_RequestErrorCopyWithImpl(this._self, this._then);

  final AlefBridgeErrorWire_RequestError _self;
  final $Res Function(AlefBridgeErrorWire_RequestError) _then;

  /// Create a copy of AlefBridgeErrorWire
  /// with the given fields replaced by the non-null parameter values.
  @pragma('vm:prefer-inline') $Res call({Object? error = null,}) {
    return _then(AlefBridgeErrorWire_RequestError(
      error: null == error ? _self.error : error // ignore: cast_nullable_to_non_nullable
      as RequestError,
    ));
  }

  /// Create a copy of AlefBridgeErrorWire
  /// with the given fields replaced by the non-null parameter values.
  @override
  @pragma('vm:prefer-inline')
  $RequestErrorCopyWith<$Res> get error {

    return $RequestErrorCopyWith<$Res>(_self.error, (value) {
      return _then(_self.copyWith(error: value));
    });
  }
}

/// @nodoc


class AlefBridgeErrorWire_InvalidInput extends AlefBridgeErrorWire {
  const AlefBridgeErrorWire_InvalidInput({required this.message}): super._();


  final  String message;

  /// Create a copy of AlefBridgeErrorWire
  /// with the given fields replaced by the non-null parameter values.
  @JsonKey(includeFromJson: false, includeToJson: false)
  @pragma('vm:prefer-inline')
  $AlefBridgeErrorWire_InvalidInputCopyWith<AlefBridgeErrorWire_InvalidInput> get copyWith => _$AlefBridgeErrorWire_InvalidInputCopyWithImpl<AlefBridgeErrorWire_InvalidInput>(this, _$identity);



  @override
  bool operator ==(Object other) {
    return identical(this, other) || (other.runtimeType == runtimeType&&other is AlefBridgeErrorWire_InvalidInput&&(identical(other.message, message) || other.message == message));
  }


  @override
  int get hashCode {
    return Object.hash(runtimeType,message);
  }

  @override
  String toString() {
    return 'AlefBridgeErrorWire.invalidInput(message: $message)';
  }


}

/// @nodoc
abstract mixin class $AlefBridgeErrorWire_InvalidInputCopyWith<$Res> implements $AlefBridgeErrorWireCopyWith<$Res> {
  factory $AlefBridgeErrorWire_InvalidInputCopyWith(AlefBridgeErrorWire_InvalidInput value, $Res Function(AlefBridgeErrorWire_InvalidInput) _then) = _$AlefBridgeErrorWire_InvalidInputCopyWithImpl;
  @useResult
  $Res call({
    String message
  });




}
/// @nodoc
class _$AlefBridgeErrorWire_InvalidInputCopyWithImpl<$Res>
implements $AlefBridgeErrorWire_InvalidInputCopyWith<$Res> {
  _$AlefBridgeErrorWire_InvalidInputCopyWithImpl(this._self, this._then);

  final AlefBridgeErrorWire_InvalidInput _self;
  final $Res Function(AlefBridgeErrorWire_InvalidInput) _then;

  /// Create a copy of AlefBridgeErrorWire
  /// with the given fields replaced by the non-null parameter values.
  @pragma('vm:prefer-inline') $Res call({Object? message = null,}) {
    return _then(AlefBridgeErrorWire_InvalidInput(
      message: null == message ? _self.message : message // ignore: cast_nullable_to_non_nullable
      as String,
    ));
  }


}

/// @nodoc
mixin _$Event {





  @override
  bool operator ==(Object other) {
    return identical(this, other) || (other.runtimeType == runtimeType&&other is Event);
  }


  @override
  int get hashCode => runtimeType.hashCode;

  @override
  String toString() {
    return 'Event()';
  }


}

/// @nodoc
class $EventCopyWith<$Res>  {
  $EventCopyWith(Event _, $Res Function(Event) __);
}


/// Adds pattern-matching-related methods to [Event].
extension EventPatterns on Event {
  /// A variant of `map` that fallback to returning `orElse`.
  ///
  /// It is equivalent to doing:
  /// ```dart
  /// switch (sealedClass) {
  ///   case final Subclass value:
  ///     return ...;
  ///   case _:
  ///     return orElse();
  /// }
  /// ```

  @optionalTypeArgs TResult maybeMap<TResult extends Object?>({TResult Function( Event_Message value)?  message,TResult Function( Event_Ready value)?  ready,required TResult orElse(),}){
    final _that = this;
    switch (_that) {
      case Event_Message() when message != null:
      return message(_that);case Event_Ready() when ready != null:
      return ready(_that);case _:
      return orElse();

    }
  }
  /// A `switch`-like method, using callbacks.
  ///
  /// Callbacks receives the raw object, upcasted.
  /// It is equivalent to doing:
  /// ```dart
  /// switch (sealedClass) {
  ///   case final Subclass value:
  ///     return ...;
  ///   case final Subclass2 value:
  ///     return ...;
  /// }
  /// ```

  @optionalTypeArgs TResult map<TResult extends Object?>({required TResult Function( Event_Message value)  message,required TResult Function( Event_Ready value)  ready,}){
    final _that = this;
    switch (_that) {
      case Event_Message():
      return message(_that);case Event_Ready():
      return ready(_that);}
  }
  /// A variant of `map` that fallback to returning `null`.
  ///
  /// It is equivalent to doing:
  /// ```dart
  /// switch (sealedClass) {
  ///   case final Subclass value:
  ///     return ...;
  ///   case _:
  ///     return null;
  /// }
  /// ```

  @optionalTypeArgs TResult? mapOrNull<TResult extends Object?>({TResult? Function( Event_Message value)?  message,TResult? Function( Event_Ready value)?  ready,}){
    final _that = this;
    switch (_that) {
      case Event_Message() when message != null:
      return message(_that);case Event_Ready() when ready != null:
      return ready(_that);case _:
      return null;

    }
  }
  /// A variant of `when` that fallback to an `orElse` callback.
  ///
  /// It is equivalent to doing:
  /// ```dart
  /// switch (sealedClass) {
  ///   case Subclass(:final field):
  ///     return ...;
  ///   case _:
  ///     return orElse();
  /// }
  /// ```

  @optionalTypeArgs TResult maybeWhen<TResult extends Object?>({TResult Function( String? tag,  bool? enabled,  PlatformInt64? count)?  message,TResult Function()?  ready,required TResult orElse(),}) {final _that = this;
    switch (_that) {
      case Event_Message() when message != null:
      return message(_that.tag,_that.enabled,_that.count);case Event_Ready() when ready != null:
      return ready();case _:
      return orElse();

    }
  }
  /// A `switch`-like method, using callbacks.
  ///
  /// As opposed to `map`, this offers destructuring.
  /// It is equivalent to doing:
  /// ```dart
  /// switch (sealedClass) {
  ///   case Subclass(:final field):
  ///     return ...;
  ///   case Subclass2(:final field2):
  ///     return ...;
  /// }
  /// ```

  @optionalTypeArgs TResult when<TResult extends Object?>({required TResult Function( String? tag,  bool? enabled,  PlatformInt64? count)  message,required TResult Function()  ready,}) {final _that = this;
    switch (_that) {
      case Event_Message():
      return message(_that.tag,_that.enabled,_that.count);case Event_Ready():
      return ready();}
  }
  /// A variant of `when` that fallback to returning `null`
  ///
  /// It is equivalent to doing:
  /// ```dart
  /// switch (sealedClass) {
  ///   case Subclass(:final field):
  ///     return ...;
  ///   case _:
  ///     return null;
  /// }
  /// ```

  @optionalTypeArgs TResult? whenOrNull<TResult extends Object?>({TResult? Function( String? tag,  bool? enabled,  PlatformInt64? count)?  message,TResult? Function()?  ready,}) {final _that = this;
    switch (_that) {
      case Event_Message() when message != null:
      return message(_that.tag,_that.enabled,_that.count);case Event_Ready() when ready != null:
      return ready();case _:
      return null;

    }
  }

}

/// @nodoc


class Event_Message extends Event {
  const Event_Message({this.tag, this.enabled, this.count}): super._();


  final  String? tag;
  final  bool? enabled;
  final  PlatformInt64? count;

  /// Create a copy of Event
  /// with the given fields replaced by the non-null parameter values.
  @JsonKey(includeFromJson: false, includeToJson: false)
  @pragma('vm:prefer-inline')
  $Event_MessageCopyWith<Event_Message> get copyWith => _$Event_MessageCopyWithImpl<Event_Message>(this, _$identity);



  @override
  bool operator ==(Object other) {
    return identical(this, other) || (other.runtimeType == runtimeType&&other is Event_Message&&(identical(other.tag, tag) || other.tag == tag)&&(identical(other.enabled, enabled) || other.enabled == enabled)&&(identical(other.count, count) || other.count == count));
  }


  @override
  int get hashCode {
    return Object.hash(runtimeType,tag,enabled,count);
  }

  @override
  String toString() {
    return 'Event.message(tag: $tag, enabled: $enabled, count: $count)';
  }


}

/// @nodoc
abstract mixin class $Event_MessageCopyWith<$Res> implements $EventCopyWith<$Res> {
  factory $Event_MessageCopyWith(Event_Message value, $Res Function(Event_Message) _then) = _$Event_MessageCopyWithImpl;
  @useResult
  $Res call({
    String? tag, bool? enabled, PlatformInt64? count
  });




}
/// @nodoc
class _$Event_MessageCopyWithImpl<$Res>
implements $Event_MessageCopyWith<$Res> {
  _$Event_MessageCopyWithImpl(this._self, this._then);

  final Event_Message _self;
  final $Res Function(Event_Message) _then;

  /// Create a copy of Event
  /// with the given fields replaced by the non-null parameter values.
  @pragma('vm:prefer-inline') $Res call({Object? tag = freezed,Object? enabled = freezed,Object? count = freezed,}) {
    return _then(Event_Message(
      tag: freezed == tag ? _self.tag : tag // ignore: cast_nullable_to_non_nullable
      as String?,enabled: freezed == enabled ? _self.enabled : enabled // ignore: cast_nullable_to_non_nullable
      as bool?,count: freezed == count ? _self.count : count // ignore: cast_nullable_to_non_nullable
      as PlatformInt64?,
    ));
  }


}

/// @nodoc


class Event_Ready extends Event {
  const Event_Ready(): super._();







  @override
  bool operator ==(Object other) {
    return identical(this, other) || (other.runtimeType == runtimeType&&other is Event_Ready);
  }


  @override
  int get hashCode => runtimeType.hashCode;

  @override
  String toString() {
    return 'Event.ready()';
  }


}




/// @nodoc
mixin _$RequestError {

  String get message; PlatformInt64? get retryAfter;
  /// Create a copy of RequestError
  /// with the given fields replaced by the non-null parameter values.
  @JsonKey(includeFromJson: false, includeToJson: false)
  @pragma('vm:prefer-inline')
  $RequestErrorCopyWith<RequestError> get copyWith => _$RequestErrorCopyWithImpl<RequestError>(this as RequestError, _$identity);



  @override
  bool operator ==(Object other) {
    final _this = this as RequestError;
    return identical(this, other) || (other.runtimeType == runtimeType&&other is RequestError&&(identical(other.message, _this.message) || other.message == _this.message)&&(identical(other.retryAfter, _this.retryAfter) || other.retryAfter == _this.retryAfter));
  }


  @override
  int get hashCode {
    final _this = this as RequestError;
    return Object.hash(runtimeType,_this.message,_this.retryAfter);
  }

  @override
  String toString() {
    final _this = this as RequestError;
    return 'RequestError(message: ${_this.message}, retryAfter: ${_this.retryAfter})';
  }


}

/// @nodoc
abstract mixin class $RequestErrorCopyWith<$Res>  {
  factory $RequestErrorCopyWith(RequestError value, $Res Function(RequestError) _then) = _$RequestErrorCopyWithImpl;
  @useResult
  $Res call({
    String message, PlatformInt64? retryAfter
  });




}
/// @nodoc
class _$RequestErrorCopyWithImpl<$Res>
implements $RequestErrorCopyWith<$Res> {
  _$RequestErrorCopyWithImpl(this._self, this._then);

  final RequestError _self;
  final $Res Function(RequestError) _then;

  /// Create a copy of RequestError
  /// with the given fields replaced by the non-null parameter values.
  @pragma('vm:prefer-inline') @override $Res call({Object? message = null,Object? retryAfter = freezed,}) {
    return _then(RequestError._(

    ));
  }

}


/// Adds pattern-matching-related methods to [RequestError].
extension RequestErrorPatterns on RequestError {
  /// A variant of `map` that fallback to returning `orElse`.
  ///
  /// It is equivalent to doing:
  /// ```dart
  /// switch (sealedClass) {
  ///   case final Subclass value:
  ///     return ...;
  ///   case _:
  ///     return orElse();
  /// }
  /// ```

  @optionalTypeArgs TResult maybeMap<TResult extends Object?>({TResult Function( RequestError_Rejected value)?  rejected,required TResult orElse(),}){
    final _that = this;
    switch (_that) {
      case RequestError_Rejected() when rejected != null:
      return rejected(_that);case _:
      return orElse();

    }
  }
  /// A `switch`-like method, using callbacks.
  ///
  /// Callbacks receives the raw object, upcasted.
  /// It is equivalent to doing:
  /// ```dart
  /// switch (sealedClass) {
  ///   case final Subclass value:
  ///     return ...;
  ///   case final Subclass2 value:
  ///     return ...;
  /// }
  /// ```

  @optionalTypeArgs TResult map<TResult extends Object?>({required TResult Function( RequestError_Rejected value)  rejected,}){
    final _that = this;
    switch (_that) {
      case RequestError_Rejected():
      return rejected(_that);}
  }
  /// A variant of `map` that fallback to returning `null`.
  ///
  /// It is equivalent to doing:
  /// ```dart
  /// switch (sealedClass) {
  ///   case final Subclass value:
  ///     return ...;
  ///   case _:
  ///     return null;
  /// }
  /// ```

  @optionalTypeArgs TResult? mapOrNull<TResult extends Object?>({TResult? Function( RequestError_Rejected value)?  rejected,}){
    final _that = this;
    switch (_that) {
      case RequestError_Rejected() when rejected != null:
      return rejected(_that);case _:
      return null;

    }
  }
  /// A variant of `when` that fallback to an `orElse` callback.
  ///
  /// It is equivalent to doing:
  /// ```dart
  /// switch (sealedClass) {
  ///   case Subclass(:final field):
  ///     return ...;
  ///   case _:
  ///     return orElse();
  /// }
  /// ```

  @optionalTypeArgs TResult maybeWhen<TResult extends Object?>({TResult Function( String message,  PlatformInt64? retryAfter)?  rejected,required TResult orElse(),}) {final _that = this;
    switch (_that) {
      case RequestError_Rejected() when rejected != null:
      return rejected(_that.message,_that.retryAfter);case _:
      return orElse();

    }
  }
  /// A `switch`-like method, using callbacks.
  ///
  /// As opposed to `map`, this offers destructuring.
  /// It is equivalent to doing:
  /// ```dart
  /// switch (sealedClass) {
  ///   case Subclass(:final field):
  ///     return ...;
  ///   case Subclass2(:final field2):
  ///     return ...;
  /// }
  /// ```

  @optionalTypeArgs TResult when<TResult extends Object?>({required TResult Function( String message,  PlatformInt64? retryAfter)  rejected,}) {final _that = this;
    switch (_that) {
      case RequestError_Rejected():
      return rejected(_that.message,_that.retryAfter);}
  }
  /// A variant of `when` that fallback to returning `null`
  ///
  /// It is equivalent to doing:
  /// ```dart
  /// switch (sealedClass) {
  ///   case Subclass(:final field):
  ///     return ...;
  ///   case _:
  ///     return null;
  /// }
  /// ```

  @optionalTypeArgs TResult? whenOrNull<TResult extends Object?>({TResult? Function( String message,  PlatformInt64? retryAfter)?  rejected,}) {final _that = this;
    switch (_that) {
      case RequestError_Rejected() when rejected != null:
      return rejected(_that.message,_that.retryAfter);case _:
      return null;

    }
  }

}

/// @nodoc


class RequestError_Rejected extends RequestError {
  const RequestError_Rejected({required this.message, this.retryAfter}): super._();


  @override final  String message;
  @override final  PlatformInt64? retryAfter;

  /// Create a copy of RequestError
  /// with the given fields replaced by the non-null parameter values.
  @override @JsonKey(includeFromJson: false, includeToJson: false)
  @pragma('vm:prefer-inline')
  $RequestError_RejectedCopyWith<RequestError_Rejected> get copyWith => _$RequestError_RejectedCopyWithImpl<RequestError_Rejected>(this, _$identity);



  @override
  bool operator ==(Object other) {
    return identical(this, other) || (other.runtimeType == runtimeType&&other is RequestError_Rejected&&(identical(other.message, message) || other.message == message)&&(identical(other.retryAfter, retryAfter) || other.retryAfter == retryAfter));
  }


  @override
  int get hashCode {
    return Object.hash(runtimeType,message,retryAfter);
  }

  @override
  String toString() {
    return 'RequestError.rejected(message: $message, retryAfter: $retryAfter)';
  }


}

/// @nodoc
abstract mixin class $RequestError_RejectedCopyWith<$Res> implements $RequestErrorCopyWith<$Res> {
  factory $RequestError_RejectedCopyWith(RequestError_Rejected value, $Res Function(RequestError_Rejected) _then) = _$RequestError_RejectedCopyWithImpl;
  @override @useResult
  $Res call({
    String message, PlatformInt64? retryAfter
  });




}
/// @nodoc
class _$RequestError_RejectedCopyWithImpl<$Res>
implements $RequestError_RejectedCopyWith<$Res> {
  _$RequestError_RejectedCopyWithImpl(this._self, this._then);

  final RequestError_Rejected _self;
  final $Res Function(RequestError_Rejected) _then;

  /// Create a copy of RequestError
  /// with the given fields replaced by the non-null parameter values.
  @override @pragma('vm:prefer-inline') $Res call({Object? message = null,Object? retryAfter = freezed,}) {
    return _then(RequestError_Rejected(
      message: null == message ? _self.message : message // ignore: cast_nullable_to_non_nullable
      as String,retryAfter: freezed == retryAfter ? _self.retryAfter : retryAfter // ignore: cast_nullable_to_non_nullable
      as PlatformInt64?,
    ));
  }


}

// dart format on
