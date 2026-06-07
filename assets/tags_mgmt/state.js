import { lower, uniqueTagNames } from "./util.js";

export const state = {
    tags: [],
    categories: [],
    stocks: [],
    untagged: [],
    tagSuggestionEnabled: false,
    selectedTagIds: new Set(),
    untaggedSelected: false,
    selectedTicker: null,
    pendingTagNames: [],
    pendingTagTicker: null,
    tagInputQuery: "",
    isUpdatingStockTags: false,
    includeAlreadyTagged: false,
    homeTab: "manual",
    lastPreview: null,
    companyProfiles: new Map(),
    tagSuggestions: new Map(),
    loadedTagSuggestions: new Set(),
    tagSuggestionPollTimers: new Map(),
    batch: {
        tickerSearch: "",
        tagState: "all",
        suggestionState: "all",
        tagSearch: "",
        visibleTickers: [],
        selectedTickers: new Set(),
        requestingSuggestions: false,
        applyingSuggestions: false,
        activeSuggestionTickers: new Set(),
        pollTimer: null,
    },
};

export function setData({ tags, categories, stocks, untagged }) {
    state.tags = tags || [];
    state.categories = categories || [];
    state.stocks = stocks || [];
    state.untagged = untagged || [];
}

export function requestRender() {
    window.dispatchEvent(new CustomEvent("tags:render"));
}

export function initialTickerFromQuery() {
    const params = new URLSearchParams(window.location.search);
    const ticker = params.get("selectedTicker") || params.get("ticker");
    return ticker ? ticker.trim().toUpperCase() : null;
}

export function applyInitialTickerSelection() {
    const ticker = initialTickerFromQuery();
    if (!ticker) return;

    state.selectedTicker = ticker;
    state.untaggedSelected = false;
    state.selectedTagIds.clear();
}

export function scrollSelectedTickerIntoView() {
    if (!state.selectedTicker) return;
    requestAnimationFrame(() => {
        const selectedRow = [...document.querySelectorAll("[data-stock-list-target='list'] [data-ticker]")]
            .find(row => row.dataset.ticker === state.selectedTicker);
        if (selectedRow) selectedRow.scrollIntoView({ block: "nearest" });
    });
}

export function clearStockTagEditor() {
    state.pendingTagNames = [];
    state.pendingTagTicker = null;
    state.tagInputQuery = "";
}

export function syncTagEditorForStock(stock) {
    if (state.pendingTagTicker === stock.ticker) return;
    state.pendingTagNames = stock.tags.map(tag => tag.name);
    state.pendingTagTicker = stock.ticker;
    state.tagInputQuery = "";
}

export function clearSelectedTags() {
    state.untaggedSelected = false;
    state.selectedTagIds.clear();
    state.selectedTicker = null;
    clearStockTagEditor();
    requestRender();
}

export function selectedTags() {
    return state.tags.filter(tag => state.selectedTagIds.has(tag.id));
}

export function selectedTagTitle() {
    const selected = selectedTags();
    if (selected.length === 1) return selected[0].name;
    return `${selected.length} selected`;
}

export function visibleStocks() {
    const queryNode = document.querySelector("[data-stock-list-target='search']");
    const query = lower(queryNode?.value || "");
    const untaggedSet = new Set(state.untagged);
    let rows = state.stocks.slice();

    if (state.untaggedSelected) {
        rows = rows.filter(stock => untaggedSet.has(stock.ticker) || stock.tags.length === 0);
    } else if (state.selectedTagIds.size) {
        rows = rows.filter(stock => stock.tags.some(tag => state.selectedTagIds.has(tag.id)));
    }

    if (query) {
        rows = rows.filter(stock =>
            lower(stock.ticker).includes(query) ||
            stock.tags.some(tag => lower(tag.name).includes(query))
        );
    }
    return rows;
}

export function latestTagUpdatedText(stock) {
    const timestamps = stock.tags
        .map(tag => tag.assigned_at ? new Date(tag.assigned_at) : null)
        .filter(date => date && !Number.isNaN(date.getTime()));
    if (!timestamps.length) return "unknown";
    const latest = new Date(Math.max(...timestamps.map(date => date.getTime())));
    return latest.toLocaleString();
}

export function tagSuggestionFor(ticker) {
    return state.tagSuggestions.get(String(ticker || "").toUpperCase()) || null;
}

export function companyProfileKey(ticker) {
    return String(ticker || "").trim().toUpperCase();
}

export function addPendingTags(names) {
    const known = state.tags.map(tag => tag.name);
    state.pendingTagNames = uniqueTagNames([...state.pendingTagNames, ...names])
        .filter(name => known.some(tag => lower(tag) === lower(name)));
    state.tagInputQuery = "";
}
