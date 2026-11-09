import { useEffect, useState } from "react";

const seconds = () => Math.floor(Date.now() / 1000);

export function useNow(intervalMs: number): number {
  const [now, setNow] = useState(seconds);
  useEffect(() => {
    const timer = window.setInterval(() => setNow(seconds()), intervalMs);
    return () => window.clearInterval(timer);
  }, [intervalMs]);
  return now;
}
