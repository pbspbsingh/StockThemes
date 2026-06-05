export function showStatus(message, type = "") {
    window.dispatchEvent(new CustomEvent("tags:status", {
        detail: { message, type },
    }));
}
