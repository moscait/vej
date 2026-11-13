package gateway

import (
	"bufio"
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"io"
	"log/slog"
	"net"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strings"
	"sync/atomic"
	"testing"
	"time"
)

type stubBackend struct {
	err   error
	calls atomic.Int32
}

func (s *stubBackend) Ensure() error {
	s.calls.Add(1)
	return s.err
}

// fakeVej serves handler on a Unix socket and returns the socket path.
func fakeVej(t *testing.T, handler http.Handler) string {
	t.Helper()
	dir, err := os.MkdirTemp("", "zvej-gw-") // short path: macOS caps socket paths
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { os.RemoveAll(dir) })
	socket := filepath.Join(dir, "vej.sock")
	ln, err := net.Listen("unix", socket)
	if err != nil {
		t.Fatal(err)
	}
	srv := &http.Server{Handler: handler}
	go srv.Serve(ln)
	t.Cleanup(func() { srv.Close() })
	return socket
}

func newGateway(t *testing.T, backend Backend, socket string) (http.Handler, *bytes.Buffer) {
	t.Helper()
	var logs bytes.Buffer
	logger := slog.New(slog.NewTextHandler(&logs, nil))
	return New(backend, socket, logger), &logs
}

// fromCaller attaches a Ziti caller the way the http.Server's ConnContext does.
func fromCaller(r *http.Request, identity string) *http.Request {
	ctx := ConnContext(r.Context(), identifiedConn{identity: identity})
	return r.WithContext(ctx)
}

type identifiedConn struct {
	net.Conn
	identity string
}

func (c identifiedConn) SourceIdentifier() string { return c.identity }

func TestForwardsRequestsToVej(t *testing.T) {
	var seen *http.Request
	var seenBody string
	socket := fakeVej(t, http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		seen = r
		body, _ := io.ReadAll(r.Body)
		seenBody = string(body)
		w.Header().Set("Content-Type", "application/json")
		w.WriteHeader(http.StatusOK)
		io.WriteString(w, `{"decision":"refund"}`)
	}))
	backend := &stubBackend{}
	gw, _ := newGateway(t, backend, socket)

	req := httptest.NewRequest(http.MethodPost, "/v1/systemone?x=1",
		strings.NewReader(`{"state":"charged twice"}`))
	req.Header.Set("Authorization", "Bearer vej_abc")
	rec := httptest.NewRecorder()
	gw.ServeHTTP(rec, fromCaller(req, "laptop"))

	if rec.Code != http.StatusOK || rec.Body.String() != `{"decision":"refund"}` {
		t.Fatalf("response = %d %q", rec.Code, rec.Body.String())
	}
	if backend.calls.Load() != 1 {
		t.Fatalf("Ensure called %d times, want 1", backend.calls.Load())
	}
	if seen.Method != http.MethodPost || seen.URL.RequestURI() != "/v1/systemone?x=1" {
		t.Fatalf("vej saw %s %s", seen.Method, seen.URL.RequestURI())
	}
	if seenBody != `{"state":"charged twice"}` {
		t.Fatalf("vej saw body %q", seenBody)
	}
	if got := seen.Header.Get("Authorization"); got != "Bearer vej_abc" {
		t.Fatalf("Authorization not passed through: %q", got)
	}
}

func TestReports503WhenVejCannotStart(t *testing.T) {
	gw, logs := newGateway(t, &stubBackend{err: errors.New("model not found: /secret/path")}, "/nonexistent.sock")

	rec := httptest.NewRecorder()
	gw.ServeHTTP(rec, fromCaller(httptest.NewRequest(http.MethodGet, "/health", nil), "laptop"))

	if rec.Code != http.StatusServiceUnavailable {
		t.Fatalf("status = %d, want 503", rec.Code)
	}
	var body map[string]string
	if err := json.Unmarshal(rec.Body.Bytes(), &body); err != nil {
		t.Fatalf("body %q is not JSON: %v", rec.Body.String(), err)
	}
	if strings.Contains(rec.Body.String(), "/secret/path") {
		t.Fatalf("internal detail leaked to the caller: %s", rec.Body.String())
	}
	if !strings.Contains(logs.String(), "/secret/path") {
		t.Fatalf("the cause must be logged for the operator:\n%s", logs.String())
	}
}

func TestReports502WhenVejDoesNotAnswer(t *testing.T) {
	gw, _ := newGateway(t, &stubBackend{}, filepath.Join(os.TempDir(), "zvej-missing.sock"))

	rec := httptest.NewRecorder()
	gw.ServeHTTP(rec, fromCaller(httptest.NewRequest(http.MethodGet, "/health", nil), "laptop"))

	if rec.Code != http.StatusBadGateway {
		t.Fatalf("status = %d, want 502", rec.Code)
	}
}

func TestAuditLogNamesTheZitiCaller(t *testing.T) {
	socket := fakeVej(t, http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
		w.WriteHeader(http.StatusTeapot)
	}))
	gw, logs := newGateway(t, &stubBackend{}, socket)

	gw.ServeHTTP(httptest.NewRecorder(),
		fromCaller(httptest.NewRequest(http.MethodPost, "/v1/batch", nil), "alice-laptop"))
	gw.ServeHTTP(httptest.NewRecorder(), httptest.NewRequest(http.MethodGet, "/v1/ps", nil))

	lines := strings.Split(strings.TrimSpace(logs.String()), "\n")
	if len(lines) != 2 {
		t.Fatalf("want one audit line per request, got:\n%s", logs.String())
	}
	for _, want := range []string{"caller=alice-laptop", "method=POST", "path=/v1/batch", "status=418"} {
		if !strings.Contains(lines[0], want) {
			t.Errorf("audit line %q lacks %q", lines[0], want)
		}
	}
	if !strings.Contains(lines[1], "caller=unknown") {
		t.Errorf("a connection without an identity must be logged as unknown: %q", lines[1])
	}
}

func TestStreamsResponsesAsVejWritesThem(t *testing.T) {
	release := make(chan struct{})
	socket := fakeVej(t, http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
		io.WriteString(w, "first\n")
		w.(http.Flusher).Flush()
		<-release
		io.WriteString(w, "second\n")
	}))
	gw, _ := newGateway(t, &stubBackend{}, socket)
	srv := httptest.NewServer(gw)
	defer srv.Close()
	defer close(release)

	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()
	req, _ := http.NewRequestWithContext(ctx, http.MethodPost, srv.URL+"/api/generate", nil)
	res, err := http.DefaultClient.Do(req)
	if err != nil {
		t.Fatal(err)
	}
	defer res.Body.Close()

	line, err := bufio.NewReader(res.Body).ReadString('\n')
	if err != nil || line != "first\n" {
		t.Fatalf("first chunk = %q, %v (must arrive before vej finishes)", line, err)
	}
}
