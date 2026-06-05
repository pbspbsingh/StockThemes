import { Controller } from "https://unpkg.com/@hotwired/stimulus@3.2.2/dist/stimulus.js";

import { api, refreshAllTagData } from "../api.js";
import {
    addPendingTags,
    clearSelectedTags,
    companyProfileKey,
    latestTagUpdatedText,
    requestRender,
    selectedTags,
    state,
    syncTagEditorForStock,
    tagSuggestionFor,
    visibleStocks,
} from "../state.js";
import { showStatus } from "../status.js";
import { escapeAttr, escapeHtml, formatDateTime, lower, uniqueTagNames } from "../util.js";

export default class extends Controller {
    static targets = ["title", "body"];

    connect() {
        this.handleRender = () => this.render();
        window.addEventListener("tags:render", this.handleRender);
        this.render();
    }

    disconnect() {
        window.removeEventListener("tags:render", this.handleRender);
        for (const timer of state.tagSuggestionPollTimers.values()) {
            clearInterval(timer);
        }
        state.tagSuggestionPollTimers.clear();
    }

    render() {
        if (this.hasTitleTarget) this.titleTarget.textContent = "AI Tagging Workflow";
        if (state.selectedTicker) {
            this.renderStockDetails();
        } else if (state.selectedTagIds.size === 1) {
            this.renderSingleTagDetails();
        } else if (state.selectedTagIds.size > 1) {
            this.renderMultiTagDetails();
        } else {
            this.renderWorkflowIntro();
        }
    }

    renderWorkflowIntro() {
        const prompt = this.buildAiPrompt(state.includeAlreadyTagged);
        this.bodyTarget.innerHTML = `
            <div class="section">
                <div class="selected-title">AI Tagging Workflow</div>
                <div class="muted">Generate a prompt for an AI agent, then paste the JSON response below.</div>
                <div class="prompt-options">
                    <label>
                        <input type="checkbox" ${state.includeAlreadyTagged ? "checked" : ""} data-action="change->workspace#toggleIncludeAlreadyTagged">
                        Include already tagged
                    </label>
                    <button class="btn primary" data-action="click->workspace#copyAiPrompt">Copy Prompt</button>
                </div>
                <textarea id="ai-prompt" readonly>${escapeHtml(prompt)}</textarea>
            </div>
            <div class="section">
                <div class="section-title">Current State</div>
                <div class="stats">
                    <span class="stat"><b>${state.untagged.length}</b> untagged stocks</span>
                    <span class="stat"><b>${state.tags.length}</b> tags</span>
                    <span class="stat"><b>${state.stocks.length}</b> total stocks</span>
                </div>
            </div>
            ${this.renderImportSection()}
        `;
    }

    renderStockDetails() {
        const stock = state.stocks.find(s => s.ticker === state.selectedTicker) || { ticker: state.selectedTicker, tags: [] };
        syncTagEditorForStock(stock);
        const latestUpdated = stock.tags.length ? latestTagUpdatedText(stock) : "";
        const tagSuggestionHtml = this.tagSuggestionSectionHtml(stock.ticker);
        this.bodyTarget.innerHTML = `
            <div class="section ticker-summary">
                <div class="ticker-summary-main">
                    <div class="selected-title">${stock.ticker}</div>
                    <div class="ticker-summary-meta">
                        <span class="stat"><b>${stock.tags.length}</b> assigned tags</span>
                    </div>
                </div>
            </div>
            <div class="section">
                <div class="section-title-row">
                    <div class="section-title">Company Profile</div>
                    <button class="btn" id="refresh-company-profile" data-action="click->workspace#refreshCompanyProfile">Refresh</button>
                </div>
                <div id="company-profile-body">
                    <div class="profile-status">Loading company profile</div>
                </div>
            </div>
            <div class="section">
                <div class="section-title-row">
                    <div class="section-title">Current Tags</div>
                    ${latestUpdated ? `<div class="muted">Last updated ${escapeHtml(latestUpdated)}</div>` : ""}
                </div>
                <div class="stock-tags">
                    ${stock.tags.length ? stock.tags.map(tag => `
                        <span class="chip"><span>${escapeHtml(tag.name)}</span></span>
                    `).join("") : '<span class="no-tags">No tags</span>'}
                </div>
            </div>
            <div class="tag-work-grid ${tagSuggestionHtml ? "" : "single"}">
                <div class="section editor-section">
                    <div class="section-title">Edit Tags</div>
                    <div class="row-actions edit-tag-controls">
                        <div class="tag-input-box">
                            <span class="pending-tags" id="pending-tags"></span>
                            <input id="stock-tag-input" placeholder="Search existing tags to add" value="${escapeAttr(state.tagInputQuery)}" data-action="input->workspace#stockTagInputChanged keydown->workspace#stockTagInputKeydown">
                        </div>
                    </div>
                    <div class="tag-suggestions" id="tag-suggestions"></div>
                    <div class="row-actions edit-tag-actions">
                        <button class="btn primary" ${state.isUpdatingStockTags ? "disabled" : ""} data-action="click->workspace#updateTagsForSelectedStock">${state.isUpdatingStockTags ? "Updating" : "Update"}</button>
                        ${this.tagSuggestionControlsHtml(stock.ticker)}
                    </div>
                </div>
                ${tagSuggestionHtml}
            </div>
            <div class="section">
                <div class="section-title">Available Tags</div>
                <div class="stock-tags available-tags" id="available-tags"></div>
            </div>
        `;

        this.loadCompanyProfile(stock.ticker);
        this.loadCachedTagSuggestion(stock.ticker);
        this.renderTagPicker();
        document.getElementById("stock-tag-input")?.focus();
    }

    renderSingleTagDetails() {
        const tagId = [...state.selectedTagIds][0];
        const tag = state.tags.find(t => t.id === tagId);
        if (!tag) {
            this.renderWorkflowIntro();
            return;
        }
        const taggedStocks = state.stocks.filter(stock => stock.tags.some(t => t.id === tag.id));
        this.bodyTarget.innerHTML = `
            <div class="section">
                <div class="selected-title">
                    <span>${escapeHtml(tag.name)}</span>
                    <button class="icon-btn" title="Rename tag" data-action="click->workspace#renameSelectedTag">✎</button>
                    <button class="icon-btn danger" title="Delete tag" data-action="click->workspace#deleteSelectedTag">×</button>
                </div>
                <div class="muted">${tag.stock_count} stocks assigned</div>
            </div>
            <div class="section">
                <div class="section-title">Category</div>
                <div class="row-actions">
                    <select id="selected-tag-category" data-current-category-id="${tag.category_id}" data-action="change->workspace#selectedTagCategoryChanged">
                        ${this.categoryOptionsHtml(tag.category_id)}
                    </select>
                    <button class="btn primary" id="update-tag-category" disabled data-action="click->workspace#moveSelectedTagToCategory">Update</button>
                </div>
            </div>
            <div class="section">
                <div class="section-title">Stocks</div>
                <div class="stock-tags">
                    ${taggedStocks.map(stock => `<span class="chip"><span>${stock.ticker}</span></span>`).join("") || '<span class="no-tags">No stocks use this tag</span>'}
                </div>
            </div>
        `;
    }

    renderMultiTagDetails() {
        const selected = selectedTags();
        const taggedStocks = visibleStocks();
        this.bodyTarget.innerHTML = `
            <div class="section">
                <div class="selected-title">
                    <span>${selected.length} tags selected</span>
                    <button class="btn" data-action="click->workspace#clearSelectedTags">Clear</button>
                </div>
                <div class="stock-tags">
                    ${selected.map(tag => `<span class="chip"><span>${escapeHtml(tag.name)}</span></span>`).join("")}
                </div>
            </div>
            <div class="section">
                <div class="section-title">Matching Stocks</div>
                <div class="stock-tags">
                    ${taggedStocks.map(stock => `<span class="chip"><span>${stock.ticker}</span></span>`).join("") || '<span class="no-tags">No stocks match these tags</span>'}
                </div>
            </div>
        `;
    }

    renderImportSection() {
        return `
            <div class="section">
                <div class="section-title">Import</div>
                <div class="muted">Paste the JSON object returned by the AI. Import replaces each ticker's current tags.</div>
            </div>
            <div class="section">
                <textarea id="import-content" placeholder='{"NVDA":["AI Infrastructure","Semiconductors"],"MSFT":["Cloud Computing","Enterprise Software"]}'></textarea>
            </div>
            <div class="section row-actions">
                <button class="btn primary" data-action="click->workspace#previewImport">Preview</button>
                <button class="btn" id="apply-import" disabled data-action="click->workspace#applyImport">Apply Import</button>
            </div>
            <div id="preview"></div>
        `;
    }

    renderPreview(preview) {
        const previewNode = document.getElementById("preview");
        if (!previewNode) return;
        const errors = preview.errors || [];
        previewNode.innerHTML = `
            <div class="stats">
                <span class="stat"><b>${preview.rows_parsed}</b> rows</span>
                <span class="stat"><b>${preview.new_tags.length}</b> unknown tags</span>
                <span class="stat"><b>${preview.mappings_to_set}</b> set</span>
                <span class="stat"><b>${preview.mappings_to_remove}</b> remove</span>
                <span class="stat"><b>${preview.unknown_tickers.length}</b> unknown tickers</span>
                <span class="stat"><b>${errors.length}</b> errors</span>
            </div>
            ${preview.new_tags.length ? `<div class="section"><div class="section-title">Unknown Tags</div><div class="stock-tags">${preview.new_tags.map(tag => `<span class="chip new"><span>${escapeHtml(tag)}</span></span>`).join("")}</div></div>` : ""}
            ${errors.length ? `<div class="errors">${errors.map(error => `<div>${error.row ? `Row ${error.row}: ` : ""}${escapeHtml(error.message)}</div>`).join("")}</div>` : ""}
            <table class="preview-table">
                <thead><tr><th>Ticker</th><th>Tags</th><th class="num">Set</th><th class="num">Remove</th></tr></thead>
                <tbody>
                    ${preview.rows.map(row => `
                        <tr>
                            <td>${row.ticker}${row.unknown_ticker ? ' <span class="chip warn"><span>unknown</span></span>' : ""}</td>
                            <td>${row.tags.length ? row.tags.map(tag => `<span class="chip ${row.new_tags.some(t => lower(t) === lower(tag)) ? "new" : ""}"><span>${escapeHtml(tag)}</span></span>`).join(" ") : '<span class="no-tags">No tags</span>'}</td>
                            <td class="num">${row.mappings_to_set}</td>
                            <td class="num">${row.mappings_to_remove}</td>
                        </tr>
                    `).join("")}
                </tbody>
            </table>
        `;
        const applyButton = document.getElementById("apply-import");
        if (applyButton) applyButton.disabled = errors.length > 0 || preview.rows_parsed === 0;
    }

    categoryOptionsHtml(selectedCategoryId) {
        return state.categories.map(category => `
            <option value="${category.id}" ${category.id === selectedCategoryId ? "selected" : ""}>${escapeHtml(category.name)}</option>
        `).join("");
    }

    tagSuggestionControlsHtml(ticker) {
        if (!state.tagSuggestionEnabled) return "";
        const suggestion = tagSuggestionFor(ticker);
        const pending = suggestion?.status === "pending";
        return `
            <button class="btn" ${pending ? "disabled" : ""} data-action="click->workspace#requestTagSuggestion">${pending ? "Suggesting" : "Suggest tags"}</button>
        `;
    }

    tagSuggestionSectionHtml(ticker) {
        if (!state.tagSuggestionEnabled) return "";
        const suggestion = tagSuggestionFor(ticker);
        if (!suggestion || suggestion.status !== "ready") return "";

        const tags = suggestion?.suggested_tags || [];
        const meta = [
            suggestion.generated_at ? `Generated ${formatDateTime(suggestion.generated_at)}` : "",
            suggestion.provider && suggestion.model ? `via ${suggestion.provider}/${suggestion.model}` : "",
        ].filter(Boolean).join(" ");
        const body = tags.length
            ? `
                <div class="model-suggestion-tags">
                    ${tags.map(tag => `<span class="chip new"><span>${escapeHtml(tag)}</span></span>`).join("")}
                </div>
            `
            : '<span class="no-tags">Model returned no matching tags</span>';
        return `
            <div class="section suggestion-section">
                <div class="section-title-row">
                    <div class="section-title">Suggested Tags</div>
                    ${meta ? `<div class="model-suggestion-meta">${escapeHtml(meta)}</div>` : ""}
                </div>
                <div class="model-suggestion-body">${body}</div>
                <div class="row-actions suggestion-actions">
                    ${tags.length ? '<button class="btn" data-action="click->workspace#applySuggestedTags">Apply</button>' : ""}
                    <button class="btn danger" data-action="click->workspace#deleteTagSuggestion">Delete suggestion</button>
                </div>
            </div>
        `;
    }

    toggleIncludeAlreadyTagged(event) {
        state.includeAlreadyTagged = event.target.checked;
        this.renderWorkflowIntro();
    }

    copyAiPrompt() {
        const value = document.getElementById("ai-prompt")?.value || "";
        navigator.clipboard.writeText(value)
            .then(() => showStatus("Copied AI prompt", "ok"))
            .catch(() => showStatus("Clipboard copy failed", "error"));
    }

    selectedTagCategoryChanged(event) {
        const current = Number(event.target.dataset.currentCategoryId);
        const updateButton = document.getElementById("update-tag-category");
        if (updateButton) updateButton.disabled = Number(event.target.value) === current;
    }

    async renameSelectedTag() {
        if (state.selectedTagIds.size !== 1) return;
        const selectedTagId = [...state.selectedTagIds][0];
        const current = state.tags.find(tag => tag.id === selectedTagId);
        const name = prompt("Rename tag", current?.name || "")?.trim();
        if (!name) return;
        try {
            const tag = await api(`/api/tags/${selectedTagId}`, { method: "PUT", body: JSON.stringify({ name }) });
            state.selectedTagIds = new Set([tag.id]);
            showStatus(`Renamed tag to ${tag.name}`, "ok");
            await refreshAllTagData();
        } catch (err) {
            showStatus(err.message, "error");
        }
    }

    async moveSelectedTagToCategory() {
        if (state.selectedTagIds.size !== 1) return;
        const select = document.getElementById("selected-tag-category");
        const categoryId = Number(select?.value);
        if (!categoryId) return;
        const selectedTagId = [...state.selectedTagIds][0];
        try {
            const tag = await api(`/api/tags/${selectedTagId}`, {
                method: "PUT",
                body: JSON.stringify({ name: "", category_id: categoryId }),
            });
            state.selectedTagIds = new Set([tag.id]);
            showStatus(`Moved ${tag.name}`, "ok");
            await refreshAllTagData();
        } catch (err) {
            showStatus(err.message, "error");
            requestRender();
        }
    }

    async deleteSelectedTag() {
        if (state.selectedTagIds.size !== 1) return;
        const selectedTagId = [...state.selectedTagIds][0];
        try {
            await api(`/api/tags/${selectedTagId}`, { method: "DELETE" });
            state.selectedTagIds.clear();
            showStatus("Deleted tag", "ok");
            await refreshAllTagData();
        } catch (err) {
            showStatus(err.message, "error");
        }
    }

    clearSelectedTags() {
        clearSelectedTags();
    }

    stockTagInputChanged(event) {
        state.tagInputQuery = event.target.value;
        this.renderTagPicker();
    }

    stockTagInputKeydown(event) {
        if (event.key !== "Enter") return;
        event.preventDefault();
        const suggestion = this.matchingAvailableTags()[0];
        if (suggestion) {
            addPendingTags([suggestion.name]);
            this.renderTagPicker();
        } else if (!state.tagInputQuery.trim()) {
            this.updateTagsForSelectedStock();
        } else {
            showStatus("Select an existing tag from suggestions or the available list", "error");
        }
    }

    removePendingTag(event) {
        state.pendingTagNames.splice(Number(event.currentTarget.dataset.removePendingTag), 1);
        this.renderTagPicker();
    }

    addSuggestedTag(event) {
        addPendingTags([event.currentTarget.dataset.suggestedTag]);
        this.renderTagPicker();
        document.getElementById("stock-tag-input")?.focus();
    }

    addAvailableTag(event) {
        if (event.currentTarget.classList.contains("disabled")) return;
        addPendingTags([event.currentTarget.dataset.availableTag]);
        this.renderTagPicker();
        document.getElementById("stock-tag-input")?.focus();
    }

    async updateTagsForSelectedStock() {
        if (state.isUpdatingStockTags) return;
        const names = uniqueTagNames(state.pendingTagNames);
        if (!state.selectedTicker) return;

        state.isUpdatingStockTags = true;
        this.renderStockDetails();
        try {
            const ok = await this.setTagsForSelectedStock(state.selectedTicker, names);
            if (ok) state.pendingTagTicker = null;
        } finally {
            state.isUpdatingStockTags = false;
            requestRender();
        }
    }

    async setTagsForSelectedStock(ticker, names) {
        try {
            const result = await api("/api/stock-tags/tags", {
                method: "PUT",
                body: JSON.stringify({ ticker, tags: names }),
            });
            showStatus(`Updated ${result.ticker}: ${result.set_tags.length} tags set, ${result.removed_tags.length} removed`, "ok");
            await refreshAllTagData();
            return true;
        } catch (err) {
            showStatus(err.message || "Failed to update tags", "error");
            return false;
        }
    }

    async previewImport() {
        try {
            state.lastPreview = await api("/api/tag-import/preview", {
                method: "POST",
                body: JSON.stringify({ content: document.getElementById("import-content")?.value || "" }),
            });
            this.renderPreview(state.lastPreview);
            const errorCount = (state.lastPreview.errors || []).length;
            showStatus(
                errorCount ? `Preview found ${errorCount} import ${errorCount === 1 ? "error" : "errors"}` : "Preview ready",
                errorCount ? "error" : "ok",
            );
        } catch (err) {
            state.lastPreview = null;
            const applyButton = document.getElementById("apply-import");
            if (applyButton) applyButton.disabled = true;
            const preview = document.getElementById("preview");
            if (preview) preview.innerHTML = "";
            showStatus(err.message, "error");
        }
    }

    async applyImport() {
        try {
            const result = await api("/api/tag-import", {
                method: "POST",
                body: JSON.stringify({ content: document.getElementById("import-content")?.value || "" }),
            });
            showStatus(`Import completed: updated ${result.mappings_set} tag assignments and removed ${result.mappings_removed} old assignments`, "ok");
            await refreshAllTagData();
            state.lastPreview = null;
            const applyButton = document.getElementById("apply-import");
            if (applyButton) applyButton.disabled = true;
        } catch (err) {
            showStatus(err.message, "error");
        }
    }

    async refreshCompanyProfile() {
        if (!state.selectedTicker) return;
        await this.loadCompanyProfile(state.selectedTicker, true);
    }

    renderCompanyProfile(ticker) {
        const node = document.getElementById("company-profile-body");
        if (!node) return;

        const key = companyProfileKey(ticker);
        const profileState = state.companyProfiles.get(key);
        const refreshBtn = document.getElementById("refresh-company-profile");
        if (refreshBtn) refreshBtn.disabled = Boolean(profileState?.loading);

        if (!profileState || profileState.loading && !profileState.profile) {
            node.innerHTML = '<div class="profile-status">Loading company profile</div>';
            return;
        }

        const profile = profileState.profile;
        const error = profileState.error
            ? `<div class="profile-status profile-error">${escapeHtml(profileState.error)}</div>`
            : "";

        if (!profile) {
            node.innerHTML = `${error}<div class="profile-status">No company profile cached</div>`;
            return;
        }

        const meta = [
            profile.sector ? `Sector: ${profile.sector}` : "",
            profile.industry ? `Industry: ${profile.industry}` : "",
            profile.fetched_at ? `Fetched: ${formatDateTime(profile.fetched_at)}` : "",
            profile.source ? `Source: ${profile.source}` : "",
        ].filter(Boolean);
        const summary = profile.summary || "";

        node.innerHTML = `
            ${error}
            <div class="profile-meta">
                ${meta.map(item => `<span class="stat">${escapeHtml(item)}</span>`).join("")}
            </div>
            <div class="profile-summary">
                ${summary ? escapeHtml(summary) : '<span class="no-tags">No description available</span>'}
            </div>
            ${profileState.loading ? '<div class="profile-status">Refreshing profile</div>' : ""}
        `;
    }

    async loadCompanyProfile(ticker, forceRefresh = false) {
        const key = companyProfileKey(ticker);
        if (!key) return;

        const cached = state.companyProfiles.get(key);
        if (!forceRefresh && cached?.profile) {
            this.renderCompanyProfile(key);
            return;
        }
        if (!forceRefresh && cached?.loading) {
            this.renderCompanyProfile(key);
            return;
        }

        state.companyProfiles.set(key, { loading: true, profile: forceRefresh ? cached?.profile : null, error: null });
        this.renderCompanyProfile(key);

        try {
            const profile = await api(`/api/company-profiles/${encodeURIComponent(key)}`, {
                method: forceRefresh ? "POST" : "GET",
            });
            state.companyProfiles.set(key, { loading: false, profile, error: null });
            if (state.selectedTicker === key) this.renderCompanyProfile(key);
            if (forceRefresh) showStatus(`Refreshed profile for ${key}`, "ok");
        } catch (err) {
            state.companyProfiles.set(key, { loading: false, profile: cached?.profile || null, error: err.message });
            if (state.selectedTicker === key) this.renderCompanyProfile(key);
            showStatus(`Failed to load profile for ${key}: ${err.message}`, "error");
        }
    }

    async requestTagSuggestion() {
        if (!state.tagSuggestionEnabled || !state.selectedTicker) return;
        await this.requestTagSuggestionForTicker(state.selectedTicker);
    }

    async requestTagSuggestionForTicker(ticker, options = {}) {
        const key = ticker.toUpperCase();
        const silent = Boolean(options.silent);
        try {
            const suggestion = await api("/api/stock-tags/suggest", {
                method: "POST",
                body: JSON.stringify({ ticker: key }),
            });
            state.loadedTagSuggestions.add(key);
            state.tagSuggestions.set(key, suggestion);
            if (suggestion.status === "ready") {
                if (!silent) showStatus(`Suggested ${suggestion.suggested_tags.length} tags for ${key}`, "ok");
            } else if (suggestion.status === "failed") {
                if (!silent) showStatus(suggestion.error || "Tag suggestion failed", "error");
            } else {
                if (!silent) showStatus(`Queued tag suggestion for ${key}`, "ok");
                this.pollTagSuggestion(key);
            }
            if (state.selectedTicker === key) this.renderStockDetails();
        } catch (err) {
            if (!silent) showStatus(err.message || "Failed to request tag suggestion", "error");
        }
    }

    async loadCachedTagSuggestion(ticker) {
        if (!state.tagSuggestionEnabled || !ticker) return;
        const key = ticker.toUpperCase();
        if (state.loadedTagSuggestions.has(key)) return;
        state.loadedTagSuggestions.add(key);
        try {
            const suggestion = await api(`/api/stock-tags/suggest/${encodeURIComponent(key)}`);
            state.tagSuggestions.set(key, suggestion);
            if (suggestion.status === "pending") {
                this.requestTagSuggestionForTicker(key, { silent: true });
            }
            if (state.selectedTicker === key) this.renderStockDetails();
        } catch (_) {
            // Missing cached suggestions are expected before the first request.
        }
    }

    pollTagSuggestion(ticker) {
        const key = ticker.toUpperCase();
        if (state.tagSuggestionPollTimers.has(key)) return;
        const timer = setInterval(async () => {
            try {
                const suggestion = await api(`/api/stock-tags/suggest/${encodeURIComponent(key)}`);
                state.tagSuggestions.set(key, suggestion);
                if (suggestion.status !== "pending") {
                    clearInterval(timer);
                    state.tagSuggestionPollTimers.delete(key);
                    if (suggestion.status === "ready") {
                        showStatus(`Suggested ${suggestion.suggested_tags.length} tags for ${key}`, "ok");
                    } else {
                        showStatus(suggestion.error || "Tag suggestion failed", "error");
                    }
                    if (state.selectedTicker === key) this.renderStockDetails();
                }
            } catch (err) {
                clearInterval(timer);
                state.tagSuggestionPollTimers.delete(key);
                showStatus(err.message || "Failed to load tag suggestion", "error");
                if (state.selectedTicker === key) this.renderStockDetails();
            }
        }, 2000);
        state.tagSuggestionPollTimers.set(key, timer);
    }

    applySuggestedTags() {
        const suggestion = tagSuggestionFor(state.selectedTicker);
        if (!suggestion || suggestion.status !== "ready") return;
        state.pendingTagNames = uniqueTagNames(suggestion.suggested_tags || []);
        state.pendingTagTicker = state.selectedTicker.toUpperCase();
        state.tagInputQuery = "";
        this.renderStockDetails();
        this.renderTagPicker();
    }

    async deleteTagSuggestion() {
        if (!state.tagSuggestionEnabled || !state.selectedTicker) return;
        const key = state.selectedTicker.toUpperCase();
        try {
            await api(`/api/stock-tags/suggest/${encodeURIComponent(key)}`, { method: "DELETE" });
            state.tagSuggestions.delete(key);
            state.loadedTagSuggestions.delete(key);
            showStatus(`Deleted tag suggestion for ${key}`, "ok");
            if (state.selectedTicker === key) this.renderStockDetails();
        } catch (err) {
            showStatus(err.message || "Failed to delete tag suggestion", "error");
        }
    }

    renderTagPicker() {
        const pendingNode = document.getElementById("pending-tags");
        if (pendingNode) {
            pendingNode.innerHTML = state.pendingTagNames.map((name, idx) => `
                <span class="chip new"><span>${escapeHtml(name)}</span><button data-remove-pending-tag="${idx}" title="Remove pending tag" data-action="click->workspace#removePendingTag">x</button></span>
            `).join("");
        }

        const suggestionsNode = document.getElementById("tag-suggestions");
        if (suggestionsNode) {
            const suggestions = this.matchingAvailableTags().slice(0, 10);
            suggestionsNode.innerHTML = state.tagInputQuery
                ? suggestions.map(tag => `<span class="chip new" data-suggested-tag="${escapeAttr(tag.name)}" data-action="click->workspace#addSuggestedTag"><span>${escapeHtml(tag.name)}</span></span>`).join("") || '<span class="no-tags">No matching existing tags</span>'
                : "";
        }

        const availableNode = document.getElementById("available-tags");
        if (availableNode) {
            const pending = new Set(state.pendingTagNames.map(lower));
            availableNode.innerHTML = state.tags.map(tag => {
                const disabled = pending.has(lower(tag.name));
                const title = disabled ? "Already selected" : "Add to edited tags";
                return `<span class="chip new ${disabled ? "disabled" : ""}" data-available-tag="${escapeAttr(tag.name)}" title="${title}" data-action="click->workspace#addAvailableTag"><span>${escapeHtml(tag.name)}</span></span>`;
            }).join("") || '<span class="no-tags">No tags defined</span>';
        }
    }

    matchingAvailableTags() {
        if (!state.selectedTicker || !state.tagInputQuery.trim()) return [];
        const pending = new Set(state.pendingTagNames.map(lower));
        const query = lower(state.tagInputQuery);
        return state.tags
            .filter(tag => !pending.has(lower(tag.name)))
            .filter(tag => lower(tag.name).includes(query));
    }

    buildAiPrompt(includeTagged) {
        const allowedTags = state.tags.map(tag => tag.name);
        const selectedStocks = includeTagged
            ? state.stocks.slice()
            : state.stocks.filter(stock => state.untagged.includes(stock.ticker) || stock.tags.length === 0);
        const inputMap = selectedStocks.length
            ? Object.fromEntries(selectedStocks.map(stock => [
                stock.ticker,
                stock.tags.length ? stock.tags.map(tag => tag.name) : [],
            ]))
            : {};

        return `You need to assign thematic tags to stocks. These tags must reflect the core business of each company. You will receive two JSON blocks.

Rules:
- Use only tags from the allowed tags JSON. Do not create new tags,
  variants, synonyms, or near-duplicates.
- Ignore existing tag values. Assign tags based solely on your knowledge
  of each company's core business.
- Assign all tags that are central to the company's business model.
  Avoid tagging peripheral or minor activities.
- If no allowed tag fits a ticker, leave its array empty.
- Return the same JSON object shape as the input, with tag arrays
  updated in place. Return JSON only - no explanation, no markdown.

Allowed tags JSON:
\`\`\`json
${JSON.stringify(allowedTags, null, 2)}
\`\`\`

Input ticker-to-tags JSON:
\`\`\`json
${JSON.stringify(inputMap, null, 2)}
\`\`\`

Return the same JSON object shape as the input ticker-to-tags JSON, with the tag arrays updated in place.
`;
    }
}
