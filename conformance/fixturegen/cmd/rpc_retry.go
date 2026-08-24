package cmd

import (
	"context"
	"io"
	"net/http"
	"strconv"
	"strings"
	"time"
)

const rpcRequestMaxAttempts = 8
const rpcRetryMaxDelay = 30 * time.Second

// doRPCGET retries transient transport failures and server responses. Archive
// RPCs commonly rate-limit the burst of historical account and balance
// queries needed by the ante corpus generator, so honor Retry-After when it is
// present and otherwise use a bounded exponential backoff.
func doRPCGET(ctx context.Context, client *http.Client, endpoint string) (*http.Response, error) {
	var lastErr error
	for attempt := 0; attempt < rpcRequestMaxAttempts; attempt++ {
		req, err := http.NewRequestWithContext(ctx, http.MethodGet, endpoint, nil)
		if err != nil {
			return nil, err
		}
		resp, err := client.Do(req)
		if err == nil && !rpcRetryableStatus(resp.StatusCode) {
			return resp, nil
		}
		if err == nil && attempt == rpcRequestMaxAttempts-1 {
			return resp, nil
		}
		if err != nil {
			lastErr = err
			if attempt == rpcRequestMaxAttempts-1 {
				return nil, lastErr
			}
		}

		delay := rpcRetryDelay(attempt, resp)
		if resp != nil && resp.Body != nil {
			_, _ = io.Copy(io.Discard, resp.Body)
			_ = resp.Body.Close()
		}
		if err := waitForRPCRetry(ctx, delay); err != nil {
			return nil, err
		}
	}
	return nil, lastErr
}

func rpcRetryableStatus(status int) bool {
	return status == http.StatusTooManyRequests ||
		status == http.StatusBadGateway ||
		status == http.StatusServiceUnavailable ||
		status == http.StatusGatewayTimeout
}

func rpcRetryDelay(attempt int, resp *http.Response) time.Duration {
	if resp != nil {
		if delay, ok := parseRetryAfter(resp.Header.Get("Retry-After"), time.Now()); ok {
			return min(delay, rpcRetryMaxDelay)
		}
	}
	delay := time.Second << min(attempt, 2)
	return delay
}

func parseRetryAfter(value string, now time.Time) (time.Duration, bool) {
	value = strings.TrimSpace(value)
	if value == "" {
		return 0, false
	}
	if seconds, err := strconv.ParseUint(value, 10, 31); err == nil {
		return time.Duration(seconds) * time.Second, true
	}
	when, err := http.ParseTime(value)
	if err != nil {
		return 0, false
	}
	if !when.After(now) {
		return 0, true
	}
	return when.Sub(now), true
}

func waitForRPCRetry(ctx context.Context, delay time.Duration) error {
	timer := time.NewTimer(delay)
	defer timer.Stop()
	select {
	case <-ctx.Done():
		return ctx.Err()
	case <-timer.C:
		return nil
	}
}
