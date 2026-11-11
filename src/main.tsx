import { lazy, StrictMode, Suspense } from "react";
import { createRoot } from "react-dom/client";
import { getCurrentWindow } from "@tauri-apps/api/window";

import "pretendard/dist/web/variable/pretendardvariable-dynamic-subset.css";
import "@/styles/tokens.css";
import "@/styles/app.css";
import { App } from "@/App";
import { trackKeyboardFocus } from "@/keyboardFocus";
import { ReviewApp } from "@/ReviewApp";
import { SyncApp } from "@/SyncApp";

trackKeyboardFocus();

const isReview = getCurrentWindow().label === "review";
const isSync = getCurrentWindow().label === "sync";
const isSetup = getCurrentWindow().label === "setup";
const isIntegrations = getCurrentWindow().label === "integrations";
const SetupApp = lazy(() => import("@/SetupApp").then((m) => ({ default: m.SetupApp })));
const IntegrationManager = lazy(() => import("@/IntegrationManager").then((m) => ({ default: m.IntegrationManager })));
document.documentElement.classList.toggle("review-window", isReview || isSync);

createRoot(document.getElementById("root")!).render(<StrictMode>{isSetup || isIntegrations ? <Suspense fallback={<div className="empty">Loading…</div>}>{isIntegrations ? <IntegrationManager /> : <SetupApp />}</Suspense> : isSync ? <SyncApp /> : isReview ? <ReviewApp /> : <App />}</StrictMode>);
