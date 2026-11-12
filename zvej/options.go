package main

import (
	"errors"
	"flag"
	"fmt"
	"io"
	"os"
	"path/filepath"
	"strings"
	"time"
)

type serveOptions struct {
	identity        string
	service         string
	vejBinary       string
	socket          string
	lazy            bool
	requireVejToken bool
	startTimeout    time.Duration
	vejArgs         []string
}

// reservedVejFlags are set by zvej itself; passing them through would either fail
// (clap rejects duplicates) or reopen the TCP port zvej exists to avoid.
var reservedVejFlags = []string{
	"--host", "--port", "-p", "--no-tcp", "--remote-socket", "--remote-token-optional",
}

const serveUsage = `Usage: zvej serve --identity FILE [options] [-- VEJ_SERVE_ARGS...]

Hosts vej as an OpenZiti service. zvej starts vej with no TCP port, serving only a
private Unix socket, and forwards requests from authenticated Ziti identities to it.
Arguments after -- go to 'vej serve', e.g. -- -m gemma-4-e2b --idle-exit 30m

Options:
`

func parseServe(args []string) (serveOptions, error) {
	fs := flag.NewFlagSet("zvej serve", flag.ContinueOnError)
	fs.SetOutput(io.Discard)
	opts := serveOptions{}
	fs.StringVar(&opts.identity, "identity", "", "enrolled OpenZiti identity file (JSON) that hosts the service")
	fs.StringVar(&opts.service, "service", "vej", "OpenZiti service name to host")
	fs.StringVar(&opts.vejBinary, "vej", "", "vej executable (default: vej next to zvej, else on PATH)")
	fs.StringVar(&opts.socket, "socket", "", "Unix socket between zvej and vej (default: a private temporary directory)")
	fs.BoolVar(&opts.lazy, "lazy", false, "start vej on the first request instead of at startup")
	fs.BoolVar(&opts.requireVejToken, "require-vej-token", false, "also require a vej bearer token (vej remote token create)")
	fs.DurationVar(&opts.startTimeout, "start-timeout", time.Minute, "how long vej may take to open its socket")
	if err := fs.Parse(args); err != nil {
		if errors.Is(err, flag.ErrHelp) {
			return opts, errHelp(fs)
		}
		return opts, fmt.Errorf("%w (run 'zvej serve --help')", err)
	}
	opts.vejArgs = fs.Args()
	return opts, opts.validate()
}

func (o serveOptions) validate() error {
	if o.identity == "" {
		return errors.New("--identity is required: an enrolled OpenZiti identity file " +
			"(see 'ziti edge enroll')")
	}
	if _, err := os.Stat(o.identity); err != nil {
		return fmt.Errorf("cannot read the identity file: %w", err)
	}
	if o.service == "" {
		return errors.New("--service must not be empty")
	}
	for _, arg := range o.vejArgs {
		name, _, _ := strings.Cut(arg, "=")
		for _, reserved := range reservedVejFlags {
			if name == reserved {
				return fmt.Errorf("%s is managed by zvej and can't be passed to vej", reserved)
			}
		}
	}
	return nil
}

// defaultVejBinary returns the vej next to the zvej executable, as shipped in the release
// archive, or "vej" to look it up on PATH.
func defaultVejBinary(zvejPath string) string {
	sibling := filepath.Join(filepath.Dir(zvejPath), "vej")
	if info, err := os.Stat(sibling); err == nil && !info.IsDir() {
		return sibling
	}
	return "vej"
}

type helpError struct{ usage string }

func (h helpError) Error() string { return h.usage }

func errHelp(fs *flag.FlagSet) error {
	var b strings.Builder
	b.WriteString(serveUsage)
	fs.SetOutput(&b)
	fs.PrintDefaults()
	return helpError{usage: b.String()}
}
