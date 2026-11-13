package backend

import (
	"context"
	"errors"
	"fmt"
	"io"
	"net"
	"net/http"
	"os"
	"os/signal"
	"path/filepath"
	"slices"
	"strings"
	"sync"
	"syscall"
	"testing"
	"time"
)

// The test binary doubles as a fake vej when FAKE_VEJ_MODE is set.
func TestMain(m *testing.M) {
	if mode := os.Getenv("FAKE_VEJ_MODE"); mode != "" {
		os.Exit(fakeVej(mode, os.Args[1:]))
	}
	os.Exit(m.Run())
}

// fakeVej mimics `vej serve --no-tcp --remote-socket PATH`:
//   - serve: serves HTTP on the socket; GET /exit makes it exit 0 (like --idle-exit)
//   - fail:  exits 1 at once (like a missing model)
//   - hang:  never opens the socket
func fakeVej(mode string, args []string) int {
	if starts := os.Getenv("FAKE_VEJ_STARTS"); starts != "" {
		f, err := os.OpenFile(starts, os.O_APPEND|os.O_CREATE|os.O_WRONLY, 0o600)
		if err == nil {
			fmt.Fprintln(f, strings.Join(args, " "))
			f.Close()
		}
	}
	switch mode {
	case "fail":
		fmt.Fprintln(os.Stderr, "Error: model not found")
		return 1
	case "hang":
		time.Sleep(time.Minute)
		return 0
	}
	i := slices.Index(args, "--remote-socket")
	if i < 0 || i+1 >= len(args) {
		return 2
	}
	socket := args[i+1]
	ln, err := net.Listen("unix", socket)
	if err != nil {
		fmt.Fprintln(os.Stderr, err)
		return 1
	}
	exit := make(chan int, 1)
	mux := http.NewServeMux()
	mux.HandleFunc("/health", func(w http.ResponseWriter, _ *http.Request) { io.WriteString(w, "ok") })
	mux.HandleFunc("/exit", func(w http.ResponseWriter, _ *http.Request) {
		io.WriteString(w, "bye")
		exit <- 0
	})
	go http.Serve(ln, mux)
	sigs := make(chan os.Signal, 1)
	signal.Notify(sigs, syscall.SIGTERM)
	code := 0
	select {
	case code = <-exit:
		time.Sleep(20 * time.Millisecond) // let the response flush
	case <-sigs:
	}
	ln.Close() // removes the socket file, as vej does
	return code
}

type harness struct {
	t      *testing.T
	b      *Backend
	socket string
	starts string
	clock  *fakeClock
}

func newHarness(t *testing.T, mode string, tweak func(*Config)) *harness {
	t.Helper()
	// Unix socket paths are capped near 104 bytes on macOS, so avoid t.TempDir().
	dir, err := os.MkdirTemp("", "zvej-test-")
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { os.RemoveAll(dir) })
	h := &harness{
		t:      t,
		socket: filepath.Join(dir, "vej.sock"),
		starts: filepath.Join(dir, "starts"),
		clock:  &fakeClock{now: time.Unix(1_000_000, 0)},
	}
	cfg := Config{
		Binary:       os.Args[0],
		Socket:       h.socket,
		ExtraArgs:    []string{"-m", "gemma-4-e2b"},
		Env:          []string{"FAKE_VEJ_MODE=" + mode, "FAKE_VEJ_STARTS=" + h.starts},
		StartTimeout: 5 * time.Second,
		RetryAfter:   10 * time.Second,
		StopTimeout:  2 * time.Second,
		Stderr:       io.Discard,
	}
	if tweak != nil {
		tweak(&cfg)
	}
	h.b = New(cfg)
	h.b.now = h.clock.Now
	t.Cleanup(func() { h.b.Stop() })
	return h
}

func (h *harness) startCount() int {
	data, err := os.ReadFile(h.starts)
	if errors.Is(err, os.ErrNotExist) {
		return 0
	}
	if err != nil {
		h.t.Fatal(err)
	}
	return strings.Count(string(data), "\n")
}

func (h *harness) startedArgs() []string {
	data, err := os.ReadFile(h.starts)
	if err != nil {
		h.t.Fatal(err)
	}
	first, _, _ := strings.Cut(string(data), "\n")
	return strings.Fields(first)
}

func (h *harness) get(path string) (string, error) {
	client := http.Client{Transport: &http.Transport{
		DialContext: func(ctx context.Context, _, _ string) (net.Conn, error) {
			var d net.Dialer
			return d.DialContext(ctx, "unix", h.socket)
		},
	}}
	res, err := client.Get("http://vej" + path)
	if err != nil {
		return "", err
	}
	defer res.Body.Close()
	body, err := io.ReadAll(res.Body)
	return string(body), err
}

func (h *harness) waitUntilStopped() {
	h.t.Helper()
	deadline := time.Now().Add(5 * time.Second)
	for h.b.Running() {
		if time.Now().After(deadline) {
			h.t.Fatal("vej did not exit")
		}
		time.Sleep(10 * time.Millisecond)
	}
}

type fakeClock struct {
	mu  sync.Mutex
	now time.Time
}

func (c *fakeClock) Now() time.Time {
	c.mu.Lock()
	defer c.mu.Unlock()
	return c.now
}

func (c *fakeClock) Advance(d time.Duration) {
	c.mu.Lock()
	defer c.mu.Unlock()
	c.now = c.now.Add(d)
}

func TestArgsServeOnlyTheRemoteSocket(t *testing.T) {
	cfg := Config{Socket: "/run/zvej/vej.sock", ExtraArgs: []string{"-m", "gemma-4-e2b"}}
	want := []string{
		"serve", "--no-tcp", "--remote-socket", "/run/zvej/vej.sock",
		"--remote-token-optional", "-m", "gemma-4-e2b",
	}
	if got := cfg.Args(); !slices.Equal(got, want) {
		t.Fatalf("Args() = %q, want %q", got, want)
	}

	cfg.RequireToken = true
	if slices.Contains(cfg.Args(), "--remote-token-optional") {
		t.Fatal("RequireToken must keep vej's bearer tokens mandatory")
	}
}

func TestEnsureStartsVejOnceForConcurrentCallers(t *testing.T) {
	h := newHarness(t, "serve", nil)

	var wg sync.WaitGroup
	errs := make(chan error, 8)
	for range 8 {
		wg.Go(func() { errs <- h.b.Ensure() })
	}
	wg.Wait()
	close(errs)
	for err := range errs {
		if err != nil {
			t.Fatal(err)
		}
	}

	if n := h.startCount(); n != 1 {
		t.Fatalf("vej started %d times, want 1", n)
	}
	if body, err := h.get("/health"); err != nil || body != "ok" {
		t.Fatalf("GET /health = %q, %v", body, err)
	}
	if !slices.Equal(h.startedArgs(), h.b.cfg.Args()) {
		t.Fatalf("vej got args %q, want %q", h.startedArgs(), h.b.cfg.Args())
	}
	if err := h.b.Ensure(); err != nil || h.startCount() != 1 {
		t.Fatalf("Ensure on a running vej: err=%v starts=%d", err, h.startCount())
	}
}

func TestEnsureRestartsVejAfterItExits(t *testing.T) {
	h := newHarness(t, "serve", nil)
	if err := h.b.Ensure(); err != nil {
		t.Fatal(err)
	}

	if _, err := h.get("/exit"); err != nil { // like vej's --idle-exit
		t.Fatal(err)
	}
	h.waitUntilStopped()

	if err := h.b.Ensure(); err != nil {
		t.Fatal(err)
	}
	if n := h.startCount(); n != 2 {
		t.Fatalf("vej started %d times, want 2", n)
	}
	if body, err := h.get("/health"); err != nil || body != "ok" {
		t.Fatalf("GET /health after restart = %q, %v", body, err)
	}
}

func TestFailedStartIsReportedAndNotRetriedUntilTheCooldownPasses(t *testing.T) {
	h := newHarness(t, "fail", nil)

	err := h.b.Ensure()
	if err == nil || !strings.Contains(err.Error(), "exited") {
		t.Fatalf("Ensure() = %v, want an 'exited' error", err)
	}
	if err := h.b.Ensure(); err == nil {
		t.Fatal("Ensure() during the cooldown must fail")
	}
	if n := h.startCount(); n != 1 {
		t.Fatalf("vej started %d times during the cooldown, want 1", n)
	}

	h.clock.Advance(11 * time.Second)
	if err := h.b.Ensure(); err == nil {
		t.Fatal("fake vej still fails")
	}
	if n := h.startCount(); n != 2 {
		t.Fatalf("vej started %d times after the cooldown, want 2", n)
	}
}

func TestStartTimesOutWhenTheSocketNeverOpens(t *testing.T) {
	h := newHarness(t, "hang", func(c *Config) { c.StartTimeout = 300 * time.Millisecond })

	err := h.b.Ensure()
	if err == nil || !strings.Contains(err.Error(), "did not open") {
		t.Fatalf("Ensure() = %v, want a timeout error", err)
	}
	if h.b.Running() {
		t.Fatal("a vej that never opened its socket must be stopped")
	}
}

func TestStopTerminatesVej(t *testing.T) {
	h := newHarness(t, "serve", nil)
	if err := h.b.Ensure(); err != nil {
		t.Fatal(err)
	}

	h.b.Stop()

	if h.b.Running() {
		t.Fatal("vej still running after Stop")
	}
	if _, err := os.Stat(h.socket); !errors.Is(err, os.ErrNotExist) {
		t.Fatalf("socket still present after Stop: %v", err)
	}
	h.b.Stop() // idempotent
}
