/**
 * Every refusal the relay makes, as JSON `{"error": code, "message": ...}`.
 *
 * A laptop puts a batch aside for good only on `bad_batch`: the batch's own
 * bytes are wrong, and sending them again would be refused again. Every
 * other answer, and any answer that is not this JSON, means "keep it and
 * try again later".
 */

export type ErrorCode =
  /** The batch's bytes are not a whole, consistent batch: see `reason`. */
  | "bad_batch"
  /** The request is not one the relay takes, whatever the batch holds. */
  | "bad_request"
  /** No token, or one the relay did not issue, or one that was revoked. */
  | "unauthorized"
  | "not_found"
  /** Not `application/x-ndjson`. */
  | "unsupported_type"
  /** The batch's `policy_version` is not one the relay takes yet. */
  | "policy_not_accepted"
  /** This install, or this address for new installs, is over its limit. */
  | "limit"
  /** Today's telemetry across every install has reached its limit. */
  | "paused"
  /** Storage did not take the batch. */
  | "storage"
  /** No storage key or pull token is set on the function. */
  | "not_configured"
  /** Refusing is switched off (`ELLA_TELEMETRY_REFUSE=off`): try later. */
  | "refusals_off"
  | "internal";

const STATUS: Record<ErrorCode, number> = {
  bad_batch: 400,
  bad_request: 400,
  unauthorized: 401,
  not_found: 404,
  unsupported_type: 415,
  policy_not_accepted: 409,
  limit: 429,
  paused: 503,
  storage: 503,
  not_configured: 503,
  refusals_off: 503,
  internal: 500,
};

export class ApiError extends Error {
  readonly code: ErrorCode;
  readonly status: number;
  /** Extra fields for the body, such as which limit was hit. */
  readonly extra: Record<string, unknown>;

  constructor(
    code: ErrorCode,
    message: string,
    extra: Record<string, unknown> = {},
    status = STATUS[code],
  ) {
    super(message);
    this.code = code;
    this.status = status;
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
