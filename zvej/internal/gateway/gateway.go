// Package gateway forwards HTTP requests that arrive over OpenZiti to vej's remote socket.
//
// OpenZiti has already authenticated each caller by the time a request gets here; the
// gateway records who it was, makes sure vej is running, and passes the request through
// unchanged. vej's remote socket only exposes the public API, so callers can't reach
// admin routes such as model switching or shutdown.
package gateway

import (
	"context"
	"encoding/json"
	"log/slog"
	"net"
	"net/http"
	"net/http/httputil"
	"time"
)

// Backend makes sure vej is running before a request is forwarded.
type Backend interface {
	Ensure() error
}

const unknownCaller = "unknown"

type callerKey struct{}

// ConnContext is an http.Server ConnContext hook that remembers the OpenZiti identity
// that opened each connection.
func ConnContext(ctx context.Context, conn net.Conn) context.Context {
	caller := unknownCaller
	if identified, ok := conn.(interface{ SourceIdentifier() string }); ok {
		if id := identified.SourceIdentifier(); id != "" {
			caller = id
		}
	}
	return context.WithValue(ctx, callerKey{}, caller)
}

func callerOf(r *http.Request) string {
	if caller, ok := r.Context().Value(callerKey{}).(string); ok {
		return caller
	}
	return unknownCaller
}

// New returns a handler that forwards requests to vej's Unix socket.
func New(backend Backend, socket string, logger *slog.Logger) http.Handler {
	proxy := &httputil.ReverseProxy{
		Rewrite: func(pr *httputil.ProxyRequest) {
			pr.Out.URL.Scheme = "http"
			pr.Out.URL.Host = "vej"
			pr.Out.Host = "vej"
		},
		Transport: &http.Transport{
			DialContext: func(ctx context.Context, _, _ string) (net.Conn, error) {
				var d net.Dialer
				return d.DialContext(ctx, "unix", socket)
			},
			MaxIdleConnsPerHost: 16,
			IdleConnTimeout:     30 * time.Second,
		},
		FlushInterval: -1, // pass streamed output through as vej writes it
		ErrorHandler: func(w http.ResponseWriter, r *http.Request, err error) {
			logger.Error("vej did not answer", "caller", callerOf(r), "path", r.URL.Path, "error", err)
			writeError(w, http.StatusBadGateway, "vej did not answer")
		},
	}
	return &gateway{backend: backend, proxy: proxy, log: logger}
}

type gateway struct {
	backend Backend
	proxy   *httputil.ReverseProxy
	log     *slog.Logger
}

func (g *gateway) ServeHTTP(w http.ResponseWriter, r *http.Request) {
	started := time.Now()
	rec := &statusRecorder{ResponseWriter: w, status: http.StatusOK}
	if err := g.backend.Ensure(); err != nil {
		g.log.Error("vej is unavailable", "caller", callerOf(r), "error", err)
		writeError(rec, http.StatusServiceUnavailable, "vej is unavailable; see the zvej logs")
	} else {
		g.proxy.ServeHTTP(rec, r)
	}
	g.log.Info("Ziti request",
		"caller", callerOf(r),
		"method", r.Method,
		"path", r.URL.Path,
		"status", rec.status,
		"elapsed_ms", time.Since(started).Milliseconds(),
	)
}

func writeError(w http.ResponseWriter, status int, message string) {
	w.Header().Set("Content-Type", "application/json")
	w.WriteHeader(status)
	_ = json.NewEncoder(w).Encode(map[string]string{"error": message})
}

// statusRecorder captures the status for the audit log. Unwrap lets the proxy flush
// through it via http.ResponseController.
type statusRecorder struct {
	http.ResponseWriter
	status      int
	wroteHeader bool
}

func (r *statusRecorder) WriteHeader(status int) {
	if !r.wroteHeader {
		r.status, r.wroteHeader = status, true
	}
	r.ResponseWriter.WriteHeader(status)
}

func (r *statusRecorder) Unwrap() http.ResponseWriter { return r.ResponseWriter }
