package com.test;

class TestLibRsException extends Exception {
    TestLibRsException(int code, String message) { super(message); }
}

class InvalidInputException extends TestLibRsException {
    InvalidInputException(String message) { super(0, message); }
}

class ConversionErrorException extends TestLibRsException {
    ConversionErrorException(String message) { super(1, message); }
}

class CoreErrorException extends TestLibRsException {
    CoreErrorException(String message) { super(2, message); }
}

class PanicException extends TestLibRsException {
    PanicException(String message) { super(3, message); }
}

class OperationCancelledException extends TestLibRsException {
    OperationCancelledException(String message) { super(5, message); }
}

class RejectedException extends TestLibRsException {
    RejectedException(String message) { super(1000, message); }
}
