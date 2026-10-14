/** Routes. */

import { lazy, Suspense } from "react";
import { BrowserRouter, Route, Routes } from "react-router-dom";
import { LiveProvider } from "./api/LiveProvider";
import { PocketProvider } from "./api/PocketProvider";
import { Shell } from "./components/Shell";
import { Loading } from "./components/ui";
import { FieldScreen } from "./screens/FieldScreen";
import { DraftProvider } from "./state/draft";

const DigPocketScreen = lazy(() => import("./screens/DigPocketScreen"));
const DigWalletScreen = lazy(() => import("./screens/DigWalletScreen"));
const RevealScreen = lazy(() => import("./screens/RevealScreen"));
const PastRoundsScreen = lazy(() => import("./screens/PastRoundsScreen"));
const RoundDetailScreen = lazy(() => import("./screens/RoundDetailScreen"));
const CheckScreen = lazy(() => import("./screens/CheckScreen"));
const WalletScreen = lazy(() => import("./screens/WalletScreen"));
const ProveWinScreen = lazy(() => import("./screens/ProveWinScreen"));
const VerifiedWinScreen = lazy(() => import("./screens/VerifiedWinScreen"));
const KeysScreen = lazy(() => import("./screens/KeysScreen"));
const CampfireScreen = lazy(() => import("./screens/CampfireScreen"));
const HowToPlayScreen = lazy(() => import("./screens/HowToPlayScreen"));
const NotFoundScreen = lazy(() => import("./screens/NotFoundScreen"));

export function App() {
  return (
    <BrowserRouter>
      <LiveProvider>
        <PocketProvider>
          <DraftProvider>
            <Shell>
              <Suspense fallback={<Loading text="Loading…" />}>
                <Routes>
                  <Route path="/" element={<FieldScreen />} />
                  <Route path="/dig/pocket" element={<DigPocketScreen />} />
                  <Route path="/dig/wallet" element={<DigWalletScreen />} />
                  <Route path="/reveal/:round" element={<RevealScreen />} />
                  <Route path="/rounds" element={<PastRoundsScreen />} />
                  <Route path="/rounds/:round" element={<RoundDetailScreen />} />
                  <Route path="/check" element={<CheckScreen />} />
                  <Route path="/check/:round" element={<CheckScreen />} />
                  <Route path="/wallet" element={<WalletScreen />} />
                  <Route path="/prove/:round" element={<ProveWinScreen />} />
                  <Route path="/win/:id" element={<VerifiedWinScreen />} />
                  <Route path="/keys" element={<KeysScreen />} />
                  <Route path="/campfire" element={<CampfireScreen />} />
                  <Route path="/how" element={<HowToPlayScreen />} />
                  <Route path="*" element={<NotFoundScreen />} />
                </Routes>
              </Suspense>
            </Shell>
          </DraftProvider>
        </PocketProvider>
      </LiveProvider>
    </BrowserRouter>
  );
}
