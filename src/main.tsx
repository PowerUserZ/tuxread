// A placeholder until the window's components exist (Task 9).
import { StrictMode } from "react";
import { createRoot } from "react-dom/client";

const root = document.getElementById("root");
if (root) {
  createRoot(root).render(
    <StrictMode>
      <h1>TuxRead</h1>
    </StrictMode>,
  );
}
