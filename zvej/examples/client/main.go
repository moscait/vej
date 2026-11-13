// Command client calls vej over OpenZiti from Go, with no tunneler.
//
//	go run ./examples/client --identity laptop.json POST /v1/systemone \
//	  '{"state":"charged twice","choices":["refund","shipping"]}'
//
// The identity needs a Dial policy for the service. In the URL, the host part is the
// OpenZiti service name, so http://vej/... reaches the service called "vej".
package main

import (
	"flag"
	"fmt"
	"io"
	"net/http"
	"os"
	"strings"
	"time"

	sdk "github.com/openziti/sdk-golang"
	"github.com/openziti/sdk-golang/ziti"
)

func main() {
	identity := flag.String("identity", "", "enrolled OpenZiti identity file (JSON)")
	service := flag.String("service", "vej", "OpenZiti service name")
	token := flag.String("token", "", "vej bearer token, if zvej runs with --require-vej-token")
	flag.Parse()
	if *identity == "" || flag.NArg() < 2 {
		fmt.Fprintln(os.Stderr, "usage: client --identity FILE METHOD PATH [JSON_BODY]")
		os.Exit(2)
	}
	if err := call(*identity, *service, *token, flag.Arg(0), flag.Arg(1), flag.Arg(2)); err != nil {
		fmt.Fprintln(os.Stderr, "client:", err)
		os.Exit(1)
	}
}

func call(identity, service, token, method, path, body string) error {
	cfg, err := ziti.NewConfigFromFile(identity)
	if err != nil {
		return fmt.Errorf("load identity: %w", err)
	}
	zitiCtx, err := ziti.NewContext(cfg)
	if err != nil {
		return fmt.Errorf("create OpenZiti context: %w", err)
	}
	defer zitiCtx.Close()

	client := sdk.NewHttpClient(zitiCtx, nil)
	client.Timeout = 2 * time.Minute
	req, err := http.NewRequest(method, "http://"+service+path, strings.NewReader(body))
	if err != nil {
		return err
	}
	if body != "" {
		req.Header.Set("Content-Type", "application/json")
	}
	if token != "" {
		req.Header.Set("Authorization", "Bearer "+token)
	}
	res, err := client.Do(req)
	if err != nil {
		return err
	}
	defer res.Body.Close()
	out, err := io.ReadAll(res.Body)
	if err != nil {
		return fmt.Errorf("read response: %w", err)
	}
	fmt.Printf("HTTP %d %s\n", res.StatusCode, strings.TrimSpace(string(out)))
	return nil
}
