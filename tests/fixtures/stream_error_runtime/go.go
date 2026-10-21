package sample

import (
	"context"
	"errors"
	"os"
)

type stream struct {
	chunks   chan int
	terminal error
}

func (s *stream) Chan() <-chan int { return s.chunks }
func (s *stream) Err() error {
	if len(s.chunks) != 0 {
		panic("stream was not consumed")
	}
	return s.terminal
}
func ChatStreamWithContext(_ context.Context) (*stream, error) {
	mode := os.Getenv("STREAM_MODE")
	if mode == "immediate" {
		return nil, errors.New("limited")
	}
	items := make(chan int, 2)
	items <- 1
	items <- 2
	close(items)
	var terminal error
	if mode == "wrong" {
		terminal = errors.New("different")
	} else if mode != "clean" {
		terminal = errors.New("limited")
	}
	return &stream{items, terminal}, nil
}
