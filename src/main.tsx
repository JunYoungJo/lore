import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { getCurrentWindow } from "@tauri-apps/api/window";

import "pretendard/dist/web/variable/pretendardvariable-dynamic-subset.css";
import "@/styles/tokens.css";
import "@/styles/app.css";
import { App } from "@/App";
import { ReviewApp } from "@/ReviewApp";

const isReview = getCurrentWindow().label === "review";
document.documentElement.classList.toggle("review-window", isReview);

createRoot(document.getElementById("root")!).render(<StrictMode>{isReview ? <ReviewApp /> : <App />}</StrictMode>);
