import type { Ctx } from "../centaur";
import { centaurBase, centaurHeaders } from "../_lib/centaur_api";
import { DOWNLOAD_TIMEOUT_MS, mimeTypeFor, requireFiles, safeFilename } from "../_lib/files";
import { query, readJson } from "../_lib/http";

/** A message that api-rs posted to the conversation. */
type Posted = {
  message_id: number;
  /** The upload path of a posted file, for example `/user_uploads/2/ab/xyz/chart.png`. */
  upload?: string;
};

/**
 * Post a Markdown message to the Zulip conversation of this session. Centaur posts the final
 * answer itself. Use this function only for a message before the answer, and only when asked.
 */
export async function post_message(args: { content: string }, ctx: Ctx): Promise<Posted> {
  if (!args.content.trim()) throw new Error("content must not be empty");
  const res = await ctx.fetch(`${centaurBase(ctx)}/api/zulip/messages`, {
    method: "POST",
    headers: { ...centaurHeaders(ctx), "content-type": "application/json" },
    body: JSON.stringify({ content: args.content }),
  });
  return posted(await readJson(res, "Centaur API /api/zulip/messages"));
}

/**
 * Upload one file to the Zulip conversation of this session and post a message that links it.
 * `path` is a file in the sandbox (the runtime sends its bytes). `filename` is the name Zulip
 * shows (default: the path's last segment). `comment` is optional text above the file link.
 * `content_type` defaults from the file extension. Zulip shows an image inline.
 *
 * @requires files
 */
export async function upload_file(
  args: { path: string; filename?: string; comment?: string; content_type?: string },
  ctx: Ctx,
): Promise<Posted> {
  const files = requireFiles(ctx, "zulip.upload_file");
  if (!(await files.stat(args.path))) throw new Error(`no such file: ${args.path}`);
  const filename = safeFilename(args.filename ?? args.path.split(/[\\/]/).pop() ?? "file");
  const content_type = args.content_type || mimeTypeFor(filename);
  const params = { filename, content_type, comment: args.comment };
  const res = await ctx.fetch(`${centaurBase(ctx)}/api/zulip/files${query(params)}`, {
    method: "POST",
    headers: { ...centaurHeaders(ctx), "content-type": content_type },
    body: { file: args.path },
    // A large file takes longer than the default call time.
    timeout_ms: DOWNLOAD_TIMEOUT_MS,
  });
  return posted(await readJson(res, "Centaur API /api/zulip/files"));
}

function posted(body: { message_id?: unknown; upload?: unknown }): Posted {
  if (typeof body.message_id !== "number") throw new Error("Centaur API returned no message_id");
  return typeof body.upload === "string"
    ? { message_id: body.message_id, upload: body.upload }
    : { message_id: body.message_id };
}
