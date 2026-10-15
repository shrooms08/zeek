/** `/api/*` → the engine (`/api/v1/board` → `/v1/board`). */
import { type Env, forward, subpath } from "../_lib/proxy";

export const onRequest: PagesFunction<Env> = (context) =>
  forward(context.request, context.env, "engine", subpath("", context.params.path), {
    cache: caches.default,
    waitUntil: (p) => {
      context.waitUntil(p);
    },
  });
