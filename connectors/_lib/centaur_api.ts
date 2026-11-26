import type { Ctx } from "../centaur";

/** The in-cluster address of api-rs. `CENTAUR_API_URL` overrides it. */
const DEFAULT_CENTAUR_API = "http://api:8000";

/** Returns the api-rs base URL without a trailing slash. */
export function centaurBase(ctx: Ctx): string {
  return (ctx.env.CENTAUR_API_URL || DEFAULT_CENTAUR_API).replace(/\/+$/, "");
}

/** Returns the headers of an api-rs request. */
export function centaurHeaders(ctx: Ctx): Record<string, string> {
  const headers: Record<string, string> = { accept: "application/json" };
  // Behind iron-proxy the API credential is injected for the API host.
  if (ctx.env.CENTAUR_API_BEARER_TOKEN) headers.authorization = `Bearer ${ctx.env.CENTAUR_API_BEARER_TOKEN}`;
  return headers;
}
