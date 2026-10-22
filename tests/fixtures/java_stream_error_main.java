package com.test;

import java.util.ArrayList;
import java.util.Iterator;
import java.util.List;
import java.util.NoSuchElementException;

public final class StreamErrorMain {
    private StreamErrorMain() {}

    public static void main(String[] args) throws Exception {
        if (args.length != 1) throw new IllegalArgumentException("expected a scenario name");
        switch (args[0]) {
            case "error_after_two_chunks" -> errorAfterTwoChunks();
            case "error_through_stream_api" -> errorThroughStreamApi();
            case "error_on_first_read" -> errorOnFirstRead();
            case "stream_rejected_on_open" -> streamRejectedOnOpen();
            case "clean_end" -> cleanEnd();
            default -> throw new IllegalArgumentException("unknown scenario: " + args[0]);
        }
        System.out.println("OK " + args[0]);
    }

    private static Iterator<ChatCompletionChunk> open() throws Exception {
        return new StreamHarness().chatStream(new ChatCompletionRequest()).iterator();
    }

    /** The server sends a, b, then an error: the caller gets a, b, then the typed error. */
    private static void errorAfterTwoChunks() throws Exception {
        NativeLib.script("a", "b", "ERR:3:server error: overloaded");
        Iterator<ChatCompletionChunk> iterator = open();
        List<String> seen = new ArrayList<>();
        Throwable failure = null;
        try {
            while (iterator.hasNext()) seen.add(iterator.next().text);
        } catch (Throwable error) {
            failure = error;
        }
        check(seen.equals(List.of("a", "b")), "chunks before the error: " + seen);
        checkTyped(failure);
        check(!iterator.hasNext(), "the iterator must be finished after the error");
        try {
            iterator.next();
            throw new AssertionError("next() after the error must throw");
        } catch (NoSuchElementException expected) {
            // The error is reported once.
        }
        check(NativeLib.streamFreeCalls == 1, "stream handle freed once, got " + NativeLib.streamFreeCalls);
    }

    private static void errorThroughStreamApi() throws Exception {
        NativeLib.script("a", "b", "ERR:3:server error: overloaded");
        List<String> seen = new ArrayList<>();
        Throwable failure = null;
        try (var stream = new StreamHarness().chatStream(new ChatCompletionRequest())) {
            stream.forEach(chunk -> seen.add(chunk.text));
        } catch (Throwable error) {
            failure = error;
        }
        check(seen.equals(List.of("a", "b")), "chunks before the error: " + seen);
        checkTyped(failure);
    }

    /** The first read fails: the error comes from the call that opens the stream. */
    private static void errorOnFirstRead() throws Exception {
        NativeLib.script("ERR:3:server error: overloaded");
        Throwable failure = null;
        try {
            open();
        } catch (Throwable error) {
            failure = error;
        }
        checkTyped(failure);
    }

    private static void streamRejectedOnOpen() throws Exception {
        NativeLib.script("START_ERR:3:server error: overloaded");
        Throwable failure = null;
        try {
            open();
        } catch (Throwable error) {
            failure = error;
        }
        checkTyped(failure);
    }

    private static void cleanEnd() throws Exception {
        NativeLib.script("a", "b", "END");
        Iterator<ChatCompletionChunk> iterator = open();
        List<String> seen = new ArrayList<>();
        while (iterator.hasNext()) seen.add(iterator.next().text);
        check(seen.equals(List.of("a", "b")), "chunks: " + seen);
        check(NativeLib.streamFreeCalls == 1, "stream handle freed once, got " + NativeLib.streamFreeCalls);
    }

    private static void checkTyped(Throwable failure) {
        check(failure != null, "the error must reach the caller");
        check(failure instanceof ServerErrorException, "wrong type: " + failure);
        check(failure.getCause() == null, "the typed error must not be wrapped: " + failure.getCause());
        check("server error: overloaded".equals(failure.getMessage()), "message: " + failure.getMessage());
        check(((ServerErrorException) failure).getStatusCode() == 500, "status lost");
    }

    private static void check(boolean condition, String message) {
        if (!condition) throw new AssertionError(message);
    }
}
