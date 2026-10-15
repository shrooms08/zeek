import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { App } from "./App";
import "./styles/index.css";

const client = new QueryClient({
  // No refetch on focus: the live stream keeps engine data fresh, and on the
  // public site every refetch is a request of its free daily allowance.
  defaultOptions: { queries: { retry: 2, refetchOnWindowFocus: false, staleTime: 5_000 } },
});

const root = document.getElementById("root");
if (root === null) throw new Error("missing #root");
createRoot(root).render(
  <StrictMode>
    <QueryClientProvider client={client}>
      <App />
    </QueryClientProvider>
  </StrictMode>,
);
