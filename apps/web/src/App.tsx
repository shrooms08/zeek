/** Routes. */

import { lazy, Suspense } from "react";
import { BrowserRouter, Route, Routes } from "react-router-dom";
import { LiveProvider } from "./api/LiveProvider";
import { PocketProvider } from "./api/PocketProvider";
import { Shell } from "./components/Shell";
import { Loading } from "./components/ui";
import { FieldScreen } from "./screens/FieldScreen";
import { DraftProvider } from "./state/draft";
import { FieldFxProvider } from "./state/fieldFx";

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
            <FieldFxProvider>
              <Shell>
                <Suspense fallback={<Loading text="Loading…" />}>
                  <Routes>
                    {/* The field stays mounted under the dig modals (child routes). */}
                    <Route element={<FieldScreen />}>
                      <Route path="/" element={null} />
                      <Route path="/dig/pocket" element={<DigPocketScreen />} />
                      <Route path="/dig/wallet" element={<DigWalletScreen />} />
                      <Route path="/how" element={<HowToPlayScreen />} />
                    </Route>
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
                    <Route path="*" element={<NotFoundScreen />} />
                  </Routes>
                </Suspense>
              </Shell>
            </FieldFxProvider>
          </DraftProvider>
        </PocketProvider>
      </LiveProvider>
    </BrowserRouter>
  );
}
