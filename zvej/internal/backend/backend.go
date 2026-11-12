// Package backend runs vej as a child process that serves only a private Unix socket.
//
// vej is started on demand and started again on the next request after it exits, whether
// it crashed or left on purpose through `--idle-exit`. Together with vej's keep_alive this
// lets zvej stay resident while vej, and the model it holds, come and go with demand.
package backend

import (
	"errors"
	"fmt"
	"io"
	"log/slog"
	"net"
	"os"
	"os/exec"
	"sync"
	"syscall"
	"time"
)

// Config describes how to run vej.
type Config struct {
	// Binary is the vej executable.
	Binary string
	// Socket is the Unix socket vej serves its remote API on.
	Socket string
	// ExtraArgs are further `vej serve` arguments, e.g. `-m gemma-4-e2b --idle-exit 30m`.
	ExtraArgs []string
	// RequireToken keeps vej's bearer tokens mandatory in addition to the OpenZiti identity.
	RequireToken bool
	// Env is added to zvej's own environment for vej.
	Env []string
	// StartTimeout bounds how long vej may take to open its socket.
	StartTimeout time.Duration
	// RetryAfter is how long a failed start is reported before vej is tried again.
	RetryAfter time.Duration
	// StopTimeout is how long vej gets to exit after SIGTERM before it is killed.
	StopTimeout time.Duration
	// Stdout and Stderr receive vej's output; nil means zvej's own.
	Stdout, Stderr io.Writer
	// Logger records starts and exits; nil means slog.Default().
	Logger *slog.Logger
}

// Args returns the `vej` command-line arguments.
func (c Config) Args() []string {
	args := []string{"serve", "--no-tcp", "--remote-socket", c.Socket}
	if !c.RequireToken {
		args = append(args, "--remote-token-optional")
	}
	return append(args, c.ExtraArgs...)
}

// Backend supervises a single vej process.
type Backend struct {
	cfg Config
	log *slog.Logger
	now func() time.Time

	mu       sync.Mutex // serialises starts and stops
	proc     *process
	failure  error
	failedAt time.Time
}

type process struct {
	cmd  *exec.Cmd
	done chan struct{}
	err  error // the exit status, valid once done is closed
}

func (p *process) exited() bool {
	select {
	case <-p.done:
		return true
	default:
		return false
	}
}

// New returns a Backend; vej is not started until Ensure is called.
func New(cfg Config) *Backend {
	if cfg.Stdout == nil {
		cfg.Stdout = os.Stdout
	}
	if cfg.Stderr == nil {
		cfg.Stderr = os.Stderr
	}
	logger := cfg.Logger
	if logger == nil {
		logger = slog.Default()
	}
	return &Backend{cfg: cfg, log: logger, now: time.Now}
}

// Ensure starts vej unless it is already running and returns once its socket accepts
// connections. Concurrent callers share a single start.
func (b *Backend) Ensure() error {
	b.mu.Lock()
	defer b.mu.Unlock()
	if b.proc != nil && !b.proc.exited() {
		return nil
	}
	b.proc = nil
	if b.failure != nil {
		if since := b.now().Sub(b.failedAt); since < b.cfg.RetryAfter {
			return fmt.Errorf("vej failed to start %s ago: %w", since.Round(time.Second), b.failure)
		}
	}
	proc, err := b.start()
	if err != nil {
		b.failure, b.failedAt = err, b.now()
		return err
	}
	b.failure = nil
	b.proc = proc
	return nil
}

// Running reports whether vej is currently running.
func (b *Backend) Running() bool {
	b.mu.Lock()
	defer b.mu.Unlock()
	return b.proc != nil && !b.proc.exited()
}

// Stop terminates vej, if running, and waits for it to exit.
func (b *Backend) Stop() {
	b.mu.Lock()
	defer b.mu.Unlock()
	if b.proc != nil {
		b.terminate(b.proc)
		b.proc = nil
	}
}

func (b *Backend) start() (*process, error) {
	if socketAccepts(b.cfg.Socket) {
		return nil, fmt.Errorf("another process is already serving %s", b.cfg.Socket)
	}
	cmd := exec.Command(b.cfg.Binary, b.cfg.Args()...)
	cmd.Env = append(os.Environ(), b.cfg.Env...)
	cmd.Stdout, cmd.Stderr = b.cfg.Stdout, b.cfg.Stderr
	if err := cmd.Start(); err != nil {
		return nil, fmt.Errorf("start %s: %w", b.cfg.Binary, err)
	}
	b.log.Info("Started vej", "pid", cmd.Process.Pid, "socket", b.cfg.Socket)
	proc := &process{cmd: cmd, done: make(chan struct{})}
	go func() {
		proc.err = cmd.Wait()
		close(proc.done)
		b.log.Info("vej exited", "pid", cmd.Process.Pid, "status", describeExit(proc.err))
	}()
	if err := b.waitUntilReady(proc); err != nil {
		b.terminate(proc)
		return nil, err
	}
	return proc, nil
}

const readyPollInterval = 20 * time.Millisecond

func (b *Backend) waitUntilReady(proc *process) error {
	deadline := time.NewTimer(b.cfg.StartTimeout)
	defer deadline.Stop()
	tick := time.NewTicker(readyPollInterval)
	defer tick.Stop()
	for {
		select {
		case <-proc.done:
			return fmt.Errorf("vej exited during startup (%s); see its output above",
				describeExit(proc.err))
		case <-deadline.C:
			return fmt.Errorf("vej did not open %s within %s", b.cfg.Socket, b.cfg.StartTimeout)
		case <-tick.C:
			if socketAccepts(b.cfg.Socket) {
				return nil
			}
		}
	}
}

// terminate asks vej to stop (SIGTERM drains in-flight requests) and kills it if it
// doesn't exit within StopTimeout.
func (b *Backend) terminate(proc *process) {
	if proc.exited() {
		return
	}
	if err := proc.cmd.Process.Signal(syscall.SIGTERM); err != nil && !errors.Is(err, os.ErrProcessDone) {
		b.log.Warn("Could not signal vej; killing it", "error", err)
		_ = proc.cmd.Process.Kill()
	}
	select {
	case <-proc.done:
	case <-time.After(b.cfg.StopTimeout):
		b.log.Warn("vej did not stop in time; killing it", "timeout", b.cfg.StopTimeout)
		_ = proc.cmd.Process.Kill()
		<-proc.done
	}
}

func socketAccepts(path string) bool {
	conn, err := net.DialTimeout("unix", path, 200*time.Millisecond)
	if err != nil {
		return false
	}
	conn.Close()
	return true
}

func describeExit(err error) string {
	if err == nil {
		return "exit status 0"
	}
	return err.Error()
}
