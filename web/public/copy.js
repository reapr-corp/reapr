// Copy buttons: each button[data-copy] copies the text of the element whose id it names.
const status = document.createElement("div");
status.className = "visually-hidden";
status.setAttribute("role", "status");
document.body.append(status);

for (const button of document.querySelectorAll("button[data-copy]")) {
  const target = document.getElementById(button.dataset.copy);
  if (!target || !navigator.clipboard) continue;
  const label = button.firstChild; // the visible "Copy" text node
  button.hidden = false;
  button.addEventListener("click", async () => {
    try {
      await navigator.clipboard.writeText(target.innerText.trim());
      label.textContent = "Copied";
      status.textContent = "Copied to the clipboard.";
    } catch {
      label.textContent = "Copy failed";
      status.textContent = "Copy failed. Select the text and copy it.";
    }
    setTimeout(() => { label.textContent = "Copy"; status.textContent = ""; }, 2000);
  });
}
