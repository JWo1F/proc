import { themeToggle, themeIconLight, themeIconDark } from "../lib/dom.js";

function getTheme() {
  return (
    localStorage.getItem("procfile-theme") ||
    (window.matchMedia("(prefers-color-scheme: dark)").matches
      ? "dark"
      : "light")
  );
}

function applyTheme(theme) {
  document.documentElement.classList.toggle("dark", theme === "dark");
  themeIconLight.classList.toggle("hidden", theme !== "dark");
  themeIconDark.classList.toggle("hidden", theme === "dark");
  localStorage.setItem("procfile-theme", theme);
}

export function initTheme() {
  applyTheme(getTheme());
  themeToggle.addEventListener("click", () => {
    applyTheme(getTheme() === "dark" ? "light" : "dark");
  });
}
