package main

import (
	"os"
	"path/filepath"
	"slices"
	"strings"
	"testing"
)

func writeIdentity(t *testing.T) string {
	t.Helper()
	path := filepath.Join(t.TempDir(), "vej-host.json")
	if err := os.WriteFile(path, []byte("{}"), 0o600); err != nil {
		t.Fatal(err)
	}
	return path
}

func TestParseServeDefaults(t *testing.T) {
	id := writeIdentity(t)

	opts, err := parseServe([]string{"--identity", id})
	if err != nil {
		t.Fatal(err)
	}

	if opts.identity != id || opts.service != "vej" {
		t.Fatalf("identity/service = %q/%q", opts.identity, opts.service)
	}
	if opts.socket != "" || opts.lazy || opts.requireVejToken {
		t.Fatalf("unexpected non-defaults: %+v", opts)
	}
	if len(opts.vejArgs) != 0 {
		t.Fatalf("vejArgs = %q, want none", opts.vejArgs)
	}
}

func TestParseServePassesArgumentsAfterDoubleDashToVej(t *testing.T) {
	id := writeIdentity(t)

	opts, err := parseServe([]string{
		"--identity", id, "--service", "vej-gpu", "--lazy", "--require-vej-token",
		"--", "-m", "gemma-4-e2b", "--idle-exit", "30m",
	})
	if err != nil {
		t.Fatal(err)
	}

	if opts.service != "vej-gpu" || !opts.lazy || !opts.requireVejToken {
		t.Fatalf("flags not applied: %+v", opts)
	}
	if want := []string{"-m", "gemma-4-e2b", "--idle-exit", "30m"}; !slices.Equal(opts.vejArgs, want) {
		t.Fatalf("vejArgs = %q, want %q", opts.vejArgs, want)
	}
}

func TestParseServeRejectsBadInput(t *testing.T) {
	id := writeIdentity(t)
	for _, tc := range []struct {
		name string
		args []string
		want string
	}{
		{"missing identity", nil, "--identity"},
		{"unreadable identity", []string{"--identity", "/nonexistent/id.json"}, "identity"},
		{"empty service", []string{"--identity", id, "--service", ""}, "--service"},
		{"vej args without --", []string{"--identity", id, "-m"}, "flag"},
		{"zvej owns the listener", []string{"--identity", id, "--", "--port", "8080"}, "--port"},
		{"zvej owns the socket", []string{"--identity", id, "--", "--remote-socket", "/tmp/x"}, "--remote-socket"},
	} {
		t.Run(tc.name, func(t *testing.T) {
			_, err := parseServe(tc.args)
			if err == nil || !strings.Contains(err.Error(), tc.want) {
				t.Fatalf("parseServe(%q) = %v, want an error mentioning %q", tc.args, err, tc.want)
			}
		})
	}
}

func TestDefaultVejBinaryPrefersTheOneNextToZvej(t *testing.T) {
	dir := t.TempDir()
	sibling := filepath.Join(dir, "vej")
	if got := defaultVejBinary(filepath.Join(dir, "zvej")); got != "vej" {
		t.Fatalf("without a sibling: %q, want PATH lookup of \"vej\"", got)
	}
	if err := os.WriteFile(sibling, []byte("#!/bin/sh\n"), 0o755); err != nil {
		t.Fatal(err)
	}
	if got := defaultVejBinary(filepath.Join(dir, "zvej")); got != sibling {
		t.Fatalf("with a sibling: %q, want %q", got, sibling)
	}
}
