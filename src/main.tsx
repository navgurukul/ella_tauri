import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import App from "./App";
import { fitPageZoom } from "./lib/zoom";
import "./styles.css";

fitPageZoom();

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <App />
  </StrictMode>,
);
