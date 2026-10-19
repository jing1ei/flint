import { StrictMode } from "react";
import "wicg-inert";
import "dialog-polyfill/dist/dialog-polyfill.css";
import { createRoot } from "react-dom/client";
import { App } from "./App";
import { initializeSkin } from "./state/skin";
import "./styles.css";

const container = document.getElementById("root");
if (container === null) throw new Error("#root is missing from index.html");

const disposeSkin = initializeSkin();
if (import.meta.hot) import.meta.hot.dispose(disposeSkin);

createRoot(container).render(
  <StrictMode>
    <App />
  </StrictMode>,
);
