import { StrictMode } from "react";
import { createRoot } from "react-dom/client";

// Fonts ship with the app (Open Font Licence); nothing is fetched from the internet.
import "@fontsource/instrument-sans/400.css";
import "@fontsource/instrument-sans/500.css";
import "@fontsource/instrument-sans/600.css";
import "@fontsource/ibm-plex-mono/400.css";
import "@fontsource/ibm-plex-mono/500.css";
import "@fontsource/doto/800.css";
import "@fontsource/doto/900.css";

import "./styles/tokens.css";
import "./styles/app.css";
import { App } from "./App";
import { finishSignIn } from "./signin";
import { applyTheme, readTheme } from "./theme";

// Before the first paint, so a chosen theme never flashes the other one.
applyTheme(readTheme());

const root = document.getElementById("root");
if (root) {
  // A sign-in link is used up before connecting, so the app connects as the person it signs in.
  void finishSignIn().then((failed) => {
    createRoot(root).render(
      <StrictMode>{failed ? <SignInFailed message={failed} /> : <App />}</StrictMode>,
    );
  });
}

/** Opened from a sign-in link that didn't work. */
function SignInFailed({ message }: { message: string }) {
  return (
    <main className="signin-failed">
      <h1>Couldn't sign in</h1>
      <p>{message}</p>
    </main>
  );
}
