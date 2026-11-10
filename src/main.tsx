import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { getCurrentWindow } from "@tauri-apps/api/window";

import "pretendard/dist/web/variable/pretendardvariable-dynamic-subset.css";
import "@/styles/tokens.css";
import "@/styles/app.css";
import { App } from "@/App";
import { ReviewApp } from "@/ReviewApp";
import { SyncApp } from "@/SyncApp";

const isReview = getCurrentWindow().label === "review";
const isSync = getCurrentWindow().label === "sync";
document.documentElement.classList.toggle("review-window", isReview || isSync);

createRoot(document.getElementById("root")!).render(<StrictMode>{isSync ? <SyncApp /> : isReview ? <ReviewApp /> : <App />}</StrictMode>);
