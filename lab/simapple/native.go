package simapple

import (
	"context"
	"encoding/binary"
	"encoding/json"
	"fmt"
	"io"
	"net"
	"os"
	"sync"
	"syscall"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	applepeer "github.com/JohanSardar/apple-bridge/lab/ipc"
)

type nativeJob struct {
	id        string
	op        string
	args      json.RawMessage
	commandID string
	ticket    string
	reply     chan nativeReply
}

type nativeReply struct {
	evidence json.RawMessage
	err      error
}

type nativeLink struct {
	conn net.Conn
	mu   sync.Mutex
}

func (l *nativeLink) write(frame any) error {
	l.mu.Lock()
	defer l.mu.Unlock()
	return writeNative(l.conn, frame)
}

func (f *fakeApple) subscribeNative(link *nativeLink) func() {
	f.nativeMu.Lock()
	if f.nativeLinks == nil {
		f.nativeLinks = map[*nativeLink]struct{}{}
	}
	f.nativeLinks[link] = struct{}{}
	f.nativeMu.Unlock()
	return func() {
		f.nativeMu.Lock()
		delete(f.nativeLinks, link)
		f.nativeMu.Unlock()
	}
}

func (f *fakeApple) broadcastChanged(source contracts.Source) {
	switch source {
	case contracts.Calendar, contracts.Contacts, contracts.Reminders:
	default:
		return
	}
	f.nativeMu.Lock()
	links := make([]*nativeLink, 0, len(f.nativeLinks))
	for link := range f.nativeLinks {
		links = append(links, link)
	}
	f.nativeMu.Unlock()
	for _, link := range links {
		go link.write(map[string]any{"t": "Changed", "source": string(source), "generation": "native/1"})
	}
}

func readNative(conn net.Conn) (map[string]json.RawMessage, error) {
	var header [4]byte
	if _, err := io.ReadFull(conn, header[:]); err != nil {
		return nil, err
	}
	size := binary.BigEndian.Uint32(header[:])
	if size == 0 || size > 1<<20 {
		return nil, contracts.ErrInvalid
	}
	body := make([]byte, size)
	if _, err := io.ReadFull(conn, body); err != nil {
		return nil, err
	}
	var frame map[string]json.RawMessage
	return frame, json.Unmarshal(body, &frame)
}

func writeNative(conn net.Conn, frame any) error {
	body, err := json.Marshal(frame)
	if err != nil {
		return err
	}
	var header [4]byte
	binary.BigEndian.PutUint32(header[:], uint32(len(body)))
	_, err = conn.Write(append(header[:], body...))
	return err
}

func (f *fakeApple) serveNative(ctx context.Context, path string) (func(), error) {
	os.Remove(path)
	listener, err := net.Listen("unix", path)
	if err != nil {
		return nil, err
	}
	if err = os.Chmod(path, 0600); err != nil {
		listener.Close()
		return nil, err
	}
	go func() {
		for {
			conn, err := listener.Accept()
			if err != nil {
				return
			}
			go f.nativeConnection(ctx, conn)
		}
	}()
	return func() { listener.Close(); os.Remove(path) }, nil
}

func (f *fakeApple) nativeConnection(ctx context.Context, conn net.Conn) {
	defer conn.Close()
	link := &nativeLink{conn: conn}
	unsubscribe := func() {}
	defer func() { unsubscribe() }()
	facts := &factSession{}
	for ctx.Err() == nil {
		frame, err := readNative(conn)
		if err != nil {
			return
		}
		var kind, id string
		json.Unmarshal(frame["t"], &kind)
		json.Unmarshal(frame["id"], &id)
		switch kind {
		case "Hello":
			facts.hello(frame)
			if link.write(map[string]any{"t": "Welcome", "protocol": "native/1", "version": 1}) != nil {
				return
			}
		case "Facts":
			if facts.subscribed == nil {
				facts.subscribed = f.subscribeNative(link)
				unsubscribe = facts.subscribed
			}
			if f.nativeFactsPage(link, facts, frame) != nil {
				return
			}
		case "Revalidate":
			if f.nativeRevalidate(link, frame) != nil {
				return
			}
		case "Effect":
			job := nativeJob{id: id, args: frame["args"], reply: make(chan nativeReply, 1)}
			var permit struct {
				CommandID string `json:"command_id"`
				Ticket    string `json:"ticket"`
			}
			json.Unmarshal(frame["op"], &job.op)
			json.Unmarshal(frame["permit"], &permit)
			job.commandID, job.ticket = permit.CommandID, permit.Ticket
			go func() {
				select {
				case f.nativeJobs <- job:
				case <-ctx.Done():
					conn.Close()
					return
				}
				var reply nativeReply
				select {
				case reply = <-job.reply:
				case <-ctx.Done():
					conn.Close()
					return
				}
				var err error
				if reply.err != nil {
					err = link.write(map[string]any{"t": "Error", "id": id, "code": "unavailable"})
				} else {
					err = link.write(map[string]any{"t": "EffectResult", "id": id, "evidence": reply.evidence})
				}
				if err != nil {
					conn.Close()
				}
			}()
		default:
			if link.write(map[string]any{"t": "Error", "id": id, "code": "unsupported"}) != nil {
				return
			}
		}
	}
}

func wirePair() (*applepeer.Wire, *applepeer.Wire, error) {
	fds, err := syscall.Socketpair(syscall.AF_UNIX, syscall.SOCK_STREAM, 0)
	if err != nil {
		return nil, nil, err
	}
	build := func(fd int) (*applepeer.Wire, error) {
		file := os.NewFile(uintptr(fd), "native-pair")
		defer file.Close()
		conn, err := net.FileConn(file)
		if err != nil {
			return nil, err
		}
		unix, ok := conn.(*net.UnixConn)
		if !ok {
			conn.Close()
			return nil, fmt.Errorf("not a unix connection")
		}
		return &applepeer.Wire{Conn: unix}, nil
	}
	writer, err := build(fds[0])
	if err != nil {
		syscall.Close(fds[1])
		return nil, nil, err
	}
	reader, err := build(fds[1])
	if err != nil {
		writer.Conn.Close()
		return nil, nil, err
	}
	return writer, reader, nil
}

func (f *fakeApple) runNative(ctx context.Context, job nativeJob, fence uint64, emit func(contracts.Capture, uint64) error, checkpoint func(contracts.Source) string) {
	evidence, err := f.nativeEffect(ctx, job, fence, emit, checkpoint)
	job.reply <- nativeReply{evidence: evidence, err: err}
}

func (f *fakeApple) nativeEffect(ctx context.Context, job nativeJob, fence uint64, emit func(contracts.Capture, uint64) error, checkpoint func(contracts.Source) string) (json.RawMessage, error) {
	if os.Getenv("AB_LAB_SESSION_EFFECTS_CONTROL") == "refuse" {
		return nil, contracts.ErrUnavailable
	}
	writer, reader, err := wirePair()
	if err != nil {
		return nil, err
	}
	defer reader.Conn.Close()
	defer writer.Conn.Close()
	frames := make(chan contracts.IPCFrame, 32)
	go func() {
		defer close(frames)
		for {
			frame, _, err := reader.Receive()
			if err != nil {
				return
			}
			frames <- frame
		}
	}()
	var wrapper struct {
		SessionQuery bool            `json:"session_query"`
		Args         json.RawMessage `json:"args"`
	}
	if json.Unmarshal(job.args, &wrapper) == nil && wrapper.SessionQuery {
		queries := newQuerySession(ctx, f, writer)
		queries.receive(contracts.IPCFrame{T: "Query", ID: job.id, Op: job.op, Args: wrapper.Args, DeadlineMS: 30000})
		select {
		case frame, ok := <-frames:
			if !ok {
				return nil, contracts.ErrUnavailable
			}
			if frame.T == "QueryResult" {
				blobs := frame.Blobs
				if blobs == nil {
					blobs = []contracts.QueryBlob{}
				}
				return json.Marshal(map[string]any{"observed": true, "native_query_result": json.RawMessage(frame.ResultJSON), "native_query_blobs": blobs})
			}
			return json.Marshal(map[string]any{"observed": false, "native_query_error": frame.Code})
		case <-time.After(35 * time.Second):
			return nil, contracts.ErrUnavailable
		case <-ctx.Done():
			return nil, ctx.Err()
		}
	}
	var command contracts.IPCFrame
	if err = json.Unmarshal(job.args, &command); err != nil {
		return nil, err
	}
	dispatch := contracts.IPCFrame{T: "Dispatch", CommandID: job.commandID, Ticket: job.ticket, OwnerFence: fence, Origin: command.Origin}
	if err = f.command(writer, dispatch, fence, emit, checkpoint); err != nil {
		return nil, err
	}
	writer.Conn.Close()
	for frame := range frames {
		switch frame.T {
		case "Outcome":
			fields := map[string]json.RawMessage{}
			if len(frame.Evidence) > 0 {
				json.Unmarshal(frame.Evidence, &fields)
			}
			fields["disposition"], _ = json.Marshal(string(frame.Disposition))
			fields["fingerprint"], _ = json.Marshal(command.RequestHash)
			return json.Marshal(fields)
		case "Rejected":
			rejected := map[string]any{"code": "unavailable", "message": "unavailable"}
			if frame.Error != nil {
				rejected["code"], rejected["message"] = frame.Error.Code, frame.Error.Message
			}
			if len(frame.Evidence) > 0 {
				rejected["evidence"] = json.RawMessage(frame.Evidence)
			}
			return json.Marshal(map[string]any{"rejected": rejected})
		}
	}
	return nil, contracts.ErrUnavailable
}
