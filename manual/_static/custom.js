// Every figure on a page opens at its full size, in a new tab, when clicked: the page shows it scaled to the column.
document.addEventListener("DOMContentLoaded", () => {
  document.querySelectorAll(".bd-article img").forEach((img) => {
    if (img.closest("a")) return;
    img.classList.add("zoomable");
    img.addEventListener("click", () => window.open(img.currentSrc || img.src, "_blank"));
  });
});
