/** `/share/*` → the engine's share pages and cards (same path). */
import { type Env, forward, subpath } from "../_lib/proxy";

export const onRequest: PagesFunction<Env> = (context) =>
  forward(context.request, context.env, "engine", subpath("/share", context.params.path));
