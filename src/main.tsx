/**
 * React bootstrap only.
 * Application state and device commands begin in App/useEngine; keeping this
 * file minimal makes startup behavior obvious to readers new to React.
 */

import React from "react";
import { createRoot } from "react-dom/client";
import { App } from "./App";
import "./styles.css";

createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
