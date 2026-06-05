import { setData } from "./state.js";

export async function api(path, options = {}) {
    try {
        const response = await fetch(path, {
            headers: { "Content-Type": "application/json", ...(options.headers || {}) },
            ...options,
        });
        if (!response.ok) {
            let message = `HTTP ${response.status}`;
            try {
                const body = await response.json();
                message = body.error || message;
            } catch (_) {}
            throw new Error(message);
        }
        if (response.status === 204) return null;
        return response.json();
    } catch (err) {
        throw new Error(err.message || "Request failed. Server may be unavailable.");
    }
}

export async function fetchAllTagData() {
    const [tags, categories, stocks, untagged] = await Promise.all([
        api("/api/tags"),
        api("/api/tag-categories"),
        api("/api/stock-tags"),
        api("/api/stock-tags/untagged"),
    ]);
    return { tags, categories, stocks, untagged };
}

export async function refreshAllTagData() {
    setData(await fetchAllTagData());
    window.dispatchEvent(new CustomEvent("tags:render"));
}
