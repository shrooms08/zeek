/** `/relay/*` → the second-source relay (`/relay/v1/chain/tip` → `/v1/chain/tip`). */
import { type Env, forward, subpath } from "../_lib/proxy";

export const onRequest: PagesFunction<Env> = (context) =>
  forward(context.request, context.env, "relay", subpath("", context.params.path));
