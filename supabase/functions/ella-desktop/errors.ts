/**
 * Every refusal the proxy makes, as JSON `{"error": code, "message": ...}`.
 * The laptop reads `error` to decide what to do: anything but `bad_request`
 * means "use the local model for now".
 */

export type ErrorCode =
  /** The request is not one the proxy forwards: see `message`. */
  | "bad_request"
  /** No token, or one the proxy did not issue, or one that was revoked. */
  | "unauthorized"
  /** This install, or this address for new installs, is over its limit. */
  | "limit"
  /** Today's spend across every install has reached the cap. */
  | "cap"
  /** No DeepSeek key is set on the function. */
  | "not_configured"
  /** DeepSeek refused the key or has no balance left: someone must act. */
  | "upstream_config"
  /** DeepSeek is too busy right now. */
  | "upstream_busy"
  /** DeepSeek failed, timed out, or could not be reached. */
  | "upstream"
  /** DeepSeek rejected the request as written. */
  | "upstream_rejected"
  | "not_found"
  | "internal";

const STATUS: Record<ErrorCode, number> = {
  bad_request: 400,
  unauthorized: 401,
  limit: 429,
  cap: 503,
  not_configured: 503,
  upstream_config: 503,
  upstream_busy: 429,
  upstream: 502,
  upstream_rejected: 400,
  not_found: 404,
  internal: 500,
};

export class ApiError extends Error {
  readonly code: ErrorCode;
  readonly status: number;
  /** Extra fields for the body, such as which limit was hit. */
  readonly extra: Record<string, unknown>;

  constructor(code: ErrorCode, message: string, extra: Record<string, unknown> = {}) {
    super(message);
    this.code = code;
    this.status = STATUS[code];
    this.extra = extra;
  }

  response(requestId: string): Response {
    return new Response(
      JSON.stringify({ error: this.code, message: this.message, ...this.extra }),
      {
        status: this.status,
        headers: { "content-type": "application/json", "x-request-id": requestId },
      },
    );
  }
}
