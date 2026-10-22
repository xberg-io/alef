package com.test;

class TestLibRsException extends Exception {
    private static final long serialVersionUID = 1L;
    private final int code;

    TestLibRsException(int code, String message) {
        super(message);
        this.code = code;
    }

    TestLibRsException(String message, Throwable cause) {
        super(message, cause);
        this.code = -1;
    }

    int getCode() { return code; }
}

class ServerErrorException extends TestLibRsException {
    private static final long serialVersionUID = 1L;
    private final int statusCode;

    ServerErrorException(String message, int statusCode) {
        super(3, message);
        this.statusCode = statusCode;
    }

    int getStatusCode() { return statusCode; }
}

class ChatCompletionRequest {}

class ChatCompletionChunk {
    final String text;

    ChatCompletionChunk(String text) { this.text = text; }
}
