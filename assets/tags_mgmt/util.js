export const UNTAGGED_TAG_ID = "__untagged__";

export function lower(value) {
    return String(value || "").toLowerCase();
}

export function escapeHtml(value) {
    return String(value)
        .replaceAll("&", "&amp;")
        .replaceAll("<", "&lt;")
        .replaceAll(">", "&gt;")
        .replaceAll('"', "&quot;")
        .replaceAll("'", "&#39;");
}

export function escapeAttr(value) {
    return escapeHtml(value);
}

export function formatDateTime(value) {
    const date = new Date(value);
    if (Number.isNaN(date.getTime())) return value;
    return date.toLocaleString();
}

export function uniqueTagNames(names) {
    return names
        .map(name => name.split(/\s+/).filter(Boolean).join(" "))
        .filter(Boolean)
        .reduce((acc, name) => {
            if (!acc.some(existing => lower(existing) === lower(name))) acc.push(name);
            return acc;
        }, []);
}
