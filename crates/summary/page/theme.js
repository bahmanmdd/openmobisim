// The Paper / Night button, as on the map: the page follows the reader's system until a theme is chosen here.
const root = document.documentElement, button = document.getElementById("theme");
const night = () => (root.dataset.theme ? root.dataset.theme === "night" : matchMedia("(prefers-color-scheme: dark)").matches);
const label = () => { button.textContent = night() ? "Paper" : "Night"; };
button.addEventListener("click", () => { root.dataset.theme = night() ? "paper" : "night"; label(); });
label();
document.body.dataset.state = "ready";
