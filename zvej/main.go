// Command zvej hosts vej as an OpenZiti service.
//
// zvej is the OpenZiti-only way to run vej: it opens no TCP port, so vej is reachable
// only by enrolled Ziti identities that the network's policies allow to dial the service.
// Plain vej contains no OpenZiti code; zvej runs it as a child process on a private Unix
// socket and forwards requests to it.
package main

import (
	"context"
	"errors"
	"fmt"
	"log/slog"
	"net"
	"net/http"
	"os"
	"os/signal"
	"path/filepath"
	"syscall"
	"time"

	"github.com/openziti/sdk-golang/ziti"

	"github.com/moscait/vej/zvej/internal/backend"
	"github.com/moscait/vej/zvej/internal/gateway"
)

// version is set at build time with -ldflags "-X main.version=...".
var version = "dev"

const (
	stopTimeout = 5 * time.Second
	retryAfter  = 10 * time.Second
)

const usage = `zvej - run vej as an OpenZiti service

Usage:
  zvej serve --identity FILE [options] [-- VEJ_SERVE_ARGS...]
  zvej version

Run 'zvej serve --help' for the serve options.
`

func main() {
	if len(os.Args) < 2 {
		fmt.Fprint(os.Stderr, usage)
		os.Exit(2)
	}
	switch os.Args[1] {
	case "serve":
		opts, err := parseServe(os.Args[2:])
		var help helpError
		if errors.As(err, &help) {
			fmt.Print(help.usage)
			return
		}
		if err != nil {
			fmt.Fprintln(os.Stderr, "zvej:", err)
			os.Exit(2)
		}
		if err := serve(opts); err != nil {
			slog.Error("zvej stopped", "error", err)
			os.Exit(1)
		}
	case "version", "--version", "-V":
		fmt.Println("zvej", version)
	case "help", "--help", "-h":
		fmt.Print(usage)
	default:
		fmt.Fprintf(os.Stderr, "zvej: unknown command %q\n\n%s", os.Args[1], usage)
		os.Exit(2)
	}
}

func serve(opts serveOptions) error {
	logger := slog.New(slog.NewTextHandler(os.Stderr, nil))
	slog.SetDefault(logger)

	socket, cleanup, err := socketPath(opts.socket)
	if err != nil {
		return err
	}
	defer cleanup()

	vejBinary := opts.vejBinary
	if vejBinary == "" {
		self, err := os.Executable()
		if err != nil {
			return fmt.Errorf("locate zvej: %w", err)
		}
		vejBinary = defaultVejBinary(self)
	}
	vej := backend.New(backend.Config{
		Binary:       vejBinary,
		Socket:       socket,
		ExtraArgs:    opts.vejArgs,
		RequireToken: opts.requireVejToken,
		StartTimeout: opts.startTimeout,
		RetryAfter:   retryAfter,
		StopTimeout:  stopTimeout,
		Logger:       logger,
	})
	defer vej.Stop()
	if !opts.lazy {
		// Fail fast on a bad model or vej path instead of on the first request.
		if err := vej.Ensure(); err != nil {
			return err
		}
	}

	listener, closeZiti, err := listenZiti(opts.identity, opts.service)
	if err != nil {
		return err
	}
	defer closeZiti()
	logger.Info("Hosting OpenZiti service", "service", opts.service, "vej", vejBinary, "lazy", opts.lazy)

	server := &http.Server{
		Handler:           gateway.New(vej, socket, logger),
		ConnContext:       gateway.ConnContext,
		ReadHeaderTimeout: 30 * time.Second,
	}
	return serveUntilSignalled(server, listener, logger)
}

func listenZiti(identityFile, service string) (net.Listener, func(), error) {
	cfg, err := ziti.NewConfigFromFile(identityFile)
	if err != nil {
		return nil, nil, fmt.Errorf("load OpenZiti identity %s: %w", identityFile, err)
	}
	zitiCtx, err := ziti.NewContext(cfg)
	if err != nil {
		return nil, nil, fmt.Errorf("create OpenZiti context: %w", err)
	}
	listener, err := zitiCtx.Listen(service)
	if err != nil {
		zitiCtx.Close()
		return nil, nil, fmt.Errorf("host OpenZiti service %q (does a Bind policy allow this "+
			"identity?): %w", service, err)
	}
	return listener, func() {
		listener.Close()
		zitiCtx.Close()
	}, nil
}

func serveUntilSignalled(server *http.Server, listener net.Listener, logger *slog.Logger) error {
	ctx, stop := signal.NotifyContext(context.Background(), os.Interrupt, syscall.SIGTERM)
	defer stop()

	served := make(chan error, 1)
	go func() { served <- server.Serve(listener) }()

	select {
	case err := <-served:
		return fmt.Errorf("OpenZiti listener closed: %w", err)
	case <-ctx.Done():
		logger.Info("Shutting down")
	}
	shutdownCtx, cancel := context.WithTimeout(context.Background(), stopTimeout)
	defer cancel()
	if err := server.Shutdown(shutdownCtx); err != nil {
		logger.Warn("In-flight requests did not finish in time", "error", err)
	}
	return nil
}

// socketPath returns the socket vej should serve on. Without an explicit path it creates
// a private (0700) directory so no other user can reach vej's socket.
func socketPath(explicit string) (string, func(), error) {
	if explicit != "" {
		abs, err := filepath.Abs(explicit)
		if err != nil {
			return "", nil, fmt.Errorf("resolve --socket: %w", err)
		}
		return abs, func() {}, nil
	}
	dir, err := os.MkdirTemp("", "zvej-")
	if err != nil {
		return "", nil, fmt.Errorf("create socket directory: %w", err)
	}
	return filepath.Join(dir, "vej.sock"), func() { os.RemoveAll(dir) }, nil
}
