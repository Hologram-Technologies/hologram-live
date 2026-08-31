const form = document.querySelector("#transform-form");
const message = document.querySelector("#message");
const result = document.querySelector("#result");

form.addEventListener("submit", async (event) => {
  event.preventDefault();
  result.textContent = "Running…";
  try {
    const response = await fetch("/_hologram/intent", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({
        version: 1,
        name: "application.invoke",
        payload: message.value,
      }),
    });
    if (!response.ok) {
      throw new Error(await response.text());
    }
    const body = await response.json();
    result.textContent = body.outputs.join("\n");
  } catch (error) {
    result.textContent = `Error: ${error.message}`;
  }
});
