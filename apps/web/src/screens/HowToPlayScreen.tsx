/** How to play: an overlay with the rules, numbers taken from the engine's config. */

import { useNavigate } from "react-router-dom";
import { useConfig } from "../api/queries";
import { Modal } from "../components/ui";
import { formatZec, zat } from "../lib/money";

export default function HowToPlayScreen() {
  const navigate = useNavigate();
  const config = useConfig();
  const p = config.data?.params;
  const close = () => {
    if (window.history.length > 1) void navigate(-1);
    else void navigate("/");
  };
  return (
    <Modal title="How to play" onClose={close} testId="how-to-play">
      <ol className="flex flex-col gap-3 text-mist list-decimal pl-5">
        <li>Pick one or more of the 25 tiles. One of them hides the treasure this round.</li>
        <li>
          Choose how much to dig on each
          {p === undefined ? "" : ` (at least ${formatZec(zat(p.min_dig_zat))} ZEC)`}. Every tile’s
          depth is public; who dug it never is.
        </li>
        <li>
          When the round closes{p === undefined ? "" : ` (every ${p.blocks_per_round} blocks)`}, a
          public random draw from drand picks the treasure tile.
        </li>
        <li>
          Everyone who dug the treasure tile shares the prize in proportion to how much they dug
          there
          {p === undefined ? "" : `, after ${p.rake_bps / 100}% goes to the Deep Cache`}.
        </li>
        <li>
          If nobody dug the treasure tile, the pot rolls into the Deep Cache.
          {p === undefined
            ? ""
            : ` Each round there is a 1 in ${p.cache_odds} chance the Deep Cache breaks open.`}
        </li>
        <li>Digs that arrive after the round closes are refunded, minus the network fee.</li>
        <li>
          Every payout is signed by two of three keyholders, and anyone can check every round.
        </li>
      </ol>
      {p !== undefined && p.network === "test" && (
        <p className="text-fog text-sm mt-4">This is testnet: TAZ has no value.</p>
      )}
      <button type="button" className="btn btn-primary w-full mt-6" onClick={close}>
        Got it
      </button>
    </Modal>
  );
}
