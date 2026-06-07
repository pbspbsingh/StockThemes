import { Controller } from "https://unpkg.com/@hotwired/stimulus@3.2.2/dist/stimulus.js";

import { api, refreshAllTagData } from "../api.js";
import {
    clearStockTagEditor,
    requestRender,
    state,
} from "../state.js";
import { showStatus } from "../status.js";
import { escapeHtml, lower, UNTAGGED_TAG_ID } from "../util.js";

export default class extends Controller {
    static targets = [
        "search",
        "list",
        "count",
        "clearSelection",
        "newCategoryForm",
        "newCategoryName",
        "newTagForm",
        "newTagCategory",
        "newTagName",
    ];

    connect() {
        this.handleRender = () => this.render();
        window.addEventListener("tags:render", this.handleRender);
        window.addEventListener("tags:sidebar-render", this.handleRender);
        this.render();
    }

    disconnect() {
        window.removeEventListener("tags:render", this.handleRender);
        window.removeEventListener("tags:sidebar-render", this.handleRender);
    }

    render() {
        this.renderNewTagCategories();
        const query = lower(this.hasSearchTarget ? this.searchTarget.value : "");
        const filtered = state.tags.filter(tag => lower(tag.name).includes(query));
        const selectedCount = state.selectedTagIds.size + (state.untaggedSelected ? 1 : 0);

        if (this.hasCountTarget) {
            this.countTarget.textContent = selectedCount
                ? `${selectedCount}/${state.tags.length + 1}`
                : `${state.tags.length + 1}`;
        }
        if (this.hasClearSelectionTarget) {
            this.clearSelectionTarget.disabled = !state.untaggedSelected && state.selectedTagIds.size === 0;
        }
        if (!this.hasListTarget) return;

        const untaggedRow = !query || "untagged".includes(query) ? `
            <div class="item ${state.untaggedSelected ? "selected" : ""}" data-tag-id="${UNTAGGED_TAG_ID}" data-action="click->tag-list#toggleTag">
                <div class="tag-row">
                    <span class="tag-name" title="Untagged">Untagged</span>
                    <span class="count">${state.untagged.length}</span>
                </div>
            </div>
        ` : "";
        const categoryRows = this.tagsByCategory(filtered).map(group => `
            <div class="category-heading">
                <span title="${escapeHtml(group.category.name)}">${escapeHtml(group.category.name)}</span>
                <span class="count">${group.category.stock_count}</span>
            </div>
            ${group.tags.map(tag => `
                <div class="item ${state.selectedTagIds.has(tag.id) ? "selected" : ""}" data-tag-id="${tag.id}" data-action="click->tag-list#toggleTag">
                    <div class="tag-row">
                        <span class="tag-name" title="${escapeHtml(tag.name)}">${escapeHtml(tag.name)}</span>
                        <span class="count">${tag.stock_count}</span>
                    </div>
                </div>
            `).join("")}
        `).join("");
        this.listTarget.innerHTML = untaggedRow + (categoryRows || (!untaggedRow ? '<div class="item"><span class="no-tags">No tags</span></div>' : ""));
    }

    tagsByCategory(filteredTags) {
        const grouped = new Map(state.categories.map(category => [category.id, { category, tags: [] }]));
        filteredTags.forEach(tag => {
            if (!grouped.has(tag.category_id)) {
                grouped.set(tag.category_id, {
                    category: { id: tag.category_id, name: "Uncategorized", stock_count: 0, sort_order: 999 },
                    tags: [],
                });
            }
            grouped.get(tag.category_id).tags.push(tag);
        });
        return [...grouped.values()].filter(group => group.tags.length);
    }

    renderNewTagCategories() {
        if (!this.hasNewTagCategoryTarget) return;
        const current = this.newTagCategoryTarget.value;
        this.newTagCategoryTarget.innerHTML = '<option value="">Select category</option>' + state.categories.map(category => (
            `<option value="${category.id}">${escapeHtml(category.name)}</option>`
        )).join("");
        if (current && state.categories.some(category => String(category.id) === current)) {
            this.newTagCategoryTarget.value = current;
        }
    }

    toggleTag(event) {
        const rawTagId = event.currentTarget.dataset.tagId;
        if (rawTagId === UNTAGGED_TAG_ID) {
            state.untaggedSelected = !state.untaggedSelected;
            state.selectedTagIds.clear();
            state.selectedTicker = null;
            clearStockTagEditor();
            requestRender();
            return;
        }

        const tagId = Number(rawTagId);
        state.untaggedSelected = false;
        if (state.selectedTagIds.has(tagId)) {
            state.selectedTagIds.delete(tagId);
        } else {
            state.selectedTagIds.add(tagId);
        }
        state.selectedTicker = null;
        clearStockTagEditor();
        requestRender();
    }

    clearSelectedTags() {
        state.untaggedSelected = false;
        state.selectedTagIds.clear();
        state.selectedTicker = null;
        clearStockTagEditor();
        requestRender();
    }

    clearTagSearch() {
        if (!this.hasSearchTarget) return;
        this.searchTarget.value = "";
        this.render();
    }

    toggleNewTagForm() {
        if (!this.hasNewTagFormTarget) return;
        this.newTagFormTarget.style.display = this.newTagFormTarget.style.display === "none" ? "grid" : "none";
        if (this.newTagFormTarget.style.display !== "none") {
            if (this.hasNewCategoryFormTarget) this.newCategoryFormTarget.style.display = "none";
            if (this.hasNewTagNameTarget) this.newTagNameTarget.focus();
        }
    }

    toggleNewCategoryForm() {
        if (!this.hasNewCategoryFormTarget) return;
        this.newCategoryFormTarget.style.display = this.newCategoryFormTarget.style.display === "none" ? "grid" : "none";
        if (this.newCategoryFormTarget.style.display !== "none") {
            if (this.hasNewTagFormTarget) this.newTagFormTarget.style.display = "none";
            if (this.hasNewCategoryNameTarget) this.newCategoryNameTarget.focus();
        }
    }

    maybeCreateTag(event) {
        if (event.key === "Enter") this.createTag();
    }

    maybeCreateCategory(event) {
        if (event.key === "Enter") this.createCategory();
    }

    async createTag() {
        if (!this.hasNewTagNameTarget || !this.hasNewTagCategoryTarget) return;
        const name = this.newTagNameTarget.value.trim();
        if (!name) return;
        const categoryId = Number(this.newTagCategoryTarget.value);
        if (!categoryId) {
            showStatus("Category is required", "error");
            this.newTagCategoryTarget.focus();
            return;
        }
        try {
            const tag = await api("/api/tags", {
                method: "POST",
                body: JSON.stringify({ name, category_id: categoryId }),
            });
            state.untaggedSelected = false;
            state.selectedTagIds = new Set([tag.id]);
            this.newTagNameTarget.value = "";
            this.newTagCategoryTarget.value = "";
            if (this.hasNewTagFormTarget) this.newTagFormTarget.style.display = "none";
            showStatus(`Created tag ${tag.name}`, "ok");
            await refreshAllTagData();
        } catch (err) {
            showStatus(err.message, "error");
        }
    }

    async createCategory() {
        if (!this.hasNewCategoryNameTarget) return;
        const name = this.newCategoryNameTarget.value.trim();
        if (!name) return;
        try {
            const category = await api("/api/tag-categories", {
                method: "POST",
                body: JSON.stringify({ name }),
            });
            this.newCategoryNameTarget.value = "";
            if (this.hasNewCategoryFormTarget) this.newCategoryFormTarget.style.display = "none";
            if (this.hasNewTagCategoryTarget) this.newTagCategoryTarget.value = String(category.id);
            showStatus(`Created category ${category.name}`, "ok");
            await refreshAllTagData();
        } catch (err) {
            showStatus(err.message, "error");
        }
    }
}
