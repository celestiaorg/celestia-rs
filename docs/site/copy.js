const button = document.getElementById("copy");
const promptText = document.getElementById("prompt");
const feedback = document.getElementById("feedback");
button.hidden = false;
button.addEventListener("click", async () => {
  try {
    await navigator.clipboard.writeText(promptText.innerText.trim());
    feedback.textContent = "Prompt copied.";
  } catch {
    const range = document.createRange();
    range.selectNodeContents(promptText);
    const selection = window.getSelection();
    selection.removeAllRanges();
    selection.addRange(range);
    feedback.textContent = "Prompt selected. Use your device’s Copy command.";
  }
});
