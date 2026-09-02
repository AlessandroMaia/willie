import React from "react";
import ReactDOM from "react-dom/client";
import { App } from "@/app/app";
import "@/styles/globals.css";

const root = document.getElementById("root");
if (root === null) {
  throw new Error("index.html must contain an element with id=root");
}

ReactDOM.createRoot(root).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
