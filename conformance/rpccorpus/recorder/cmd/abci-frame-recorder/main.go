// Command abci-frame-recorder proxies local ABCI socket connections and
// records every framed payload in abci_server::record format.
package main

import (
	"bufio"
	"encoding/binary"
	"fmt"
	"io"
	"net"
	"os"
	"sync"
)

type recording struct {
	mu sync.Mutex
	w  *bufio.Writer
}

func (r *recording) append(connection uint32, response bool, payload []byte) error {
	r.mu.Lock()
	defer r.mu.Unlock()
	direction := byte(0)
	if response {
		direction = 1
	}
	var header [9]byte
	binary.LittleEndian.PutUint32(header[0:4], connection)
	header[4] = direction
	binary.LittleEndian.PutUint32(header[5:9], uint32(len(payload)))
	if _, err := r.w.Write(header[:]); err != nil {
		return err
	}
	if _, err := r.w.Write(payload); err != nil {
		return err
	}
	return r.w.Flush()
}

func readFrame(r *bufio.Reader) ([]byte, []byte, error) {
	length, err := binary.ReadUvarint(r)
	if err != nil {
		return nil, nil, err
	}
	if length > 64<<20 {
		return nil, nil, fmt.Errorf("ABCI frame exceeds 64 MiB")
	}
	payload := make([]byte, int(length))
	if _, err := io.ReadFull(r, payload); err != nil {
		return nil, nil, err
	}
	prefix := binary.AppendUvarint(nil, length)
	return prefix, payload, nil
}

func pump(connection uint32, response bool, dst net.Conn, src net.Conn, record *recording, done chan<- error) {
	r := bufio.NewReader(src)
	for {
		prefix, payload, err := readFrame(r)
		if err != nil {
			done <- err
			return
		}
		if err := record.append(connection, response, payload); err != nil {
			done <- err
			return
		}
		frame := append(prefix, payload...)
		for len(frame) != 0 {
			n, err := dst.Write(frame)
			if err != nil {
				done <- err
				return
			}
			if n == 0 {
				done <- io.ErrShortWrite
				return
			}
			frame = frame[n:]
		}
	}
}

func proxy(connection uint32, client net.Conn, upstream string, record *recording) {
	server, err := net.Dial("tcp", upstream)
	if err != nil {
		_ = client.Close()
		return
	}
	done := make(chan error, 2)
	go pump(connection, false, server, client, record, done)
	go pump(connection, true, client, server, record, done)
	<-done
	_ = client.Close()
	_ = server.Close()
}

func loopback(address string) error {
	host, _, err := net.SplitHostPort(address)
	if err != nil {
		return err
	}
	ip := net.ParseIP(host)
	if ip == nil || !ip.IsLoopback() {
		return fmt.Errorf("address %q is not explicit loopback", address)
	}
	return nil
}

func run(args []string) error {
	if len(args) != 3 {
		return fmt.Errorf("usage: abci-frame-recorder LISTEN UPSTREAM OUTPUT")
	}
	if err := loopback(args[0]); err != nil {
		return err
	}
	if err := loopback(args[1]); err != nil {
		return err
	}
	out, err := os.Create(args[2])
	if err != nil {
		return err
	}
	defer out.Close()
	listener, err := net.Listen("tcp", args[0])
	if err != nil {
		return err
	}
	defer listener.Close()
	record := &recording{w: bufio.NewWriter(out)}
	var connection uint32
	for {
		client, err := listener.Accept()
		if err != nil {
			return err
		}
		connection++
		go proxy(connection, client, args[1], record)
	}
}

func main() {
	if err := run(os.Args[1:]); err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(1)
	}
}
