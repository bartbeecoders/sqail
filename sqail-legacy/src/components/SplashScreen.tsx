import { useState, useEffect } from "react";
import splashImage from "../assets/splash.png";

interface SplashScreenProps {
  onComplete: () => void;
}

const STEPS = [
  "Initializing application...",
  "Loading editor...",
  "Restoring session...",
  "Ready",
];

const STEP_DURATION = 400;

export default function SplashScreen({ onComplete }: SplashScreenProps) {
  const [progress, setProgress] = useState(0);
  const [stepIdx, setStepIdx] = useState(0);
  const [fadeOut, setFadeOut] = useState(false);

  useEffect(() => {
    let frame: number;
    const start = performance.now();
    const totalDuration = STEPS.length * STEP_DURATION;

    const tick = (now: number) => {
      const elapsed = now - start;
      const pct = Math.min(elapsed / totalDuration, 1);
      setProgress(pct);
      setStepIdx(Math.min(Math.floor(pct * STEPS.length), STEPS.length - 1));

      if (pct < 1) {
        frame = requestAnimationFrame(tick);
      } else {
        // Hold briefly at 100%, then fade out
        setTimeout(() => {
          setFadeOut(true);
          setTimeout(onComplete, 400);
        }, 300);
      }
    };

    frame = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(frame);
  }, [onComplete]);

  return (
    <div
      className={`splash-root ${fadeOut ? "splash-fade-out" : ""}`}
    >
      <div className="splash-content">
        <img
          src={splashImage}
          alt="SQaiL"
          className="splash-logo"
        />
        <div className="splash-version">
          v{__APP_VERSION__}
        </div>
      </div>

      <div className="splash-footer">
        <div className="splash-bar-track">
          <div
            className="splash-bar-fill"
            style={{ width: `${progress * 100}%` }}
          />
        </div>
        <div className="splash-step">
          {STEPS[stepIdx]}
        </div>
      </div>
    </div>
  );
}
