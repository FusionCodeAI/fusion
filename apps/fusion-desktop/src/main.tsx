import "./preamble";
import React from "react";
import { createRoot } from "react-dom/client";
import { App } from "./App";
import "./index.css";

console.log("[main.tsx] executing createRoot now!");
createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>
);
