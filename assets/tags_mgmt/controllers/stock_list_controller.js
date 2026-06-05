import { Controller } from "https://unpkg.com/@hotwired/stimulus@3.2.2/dist/stimulus.js";

import {
    clearStockTagEditor,
    requestRender,
    selectedTagTitle,
    state,
    visibleStocks,
} from "../state.js";
import { escapeHtml } from "../util.js";

export default class extends Controller {
    static targets = ["search", "title", "count", "list"];

    connect() {
        this.handleRender = () => this.render();
        window.addEventListener("tags:render", this.handleRender);
        this.render();
    }

    disconnect() {
        window.removeEventListener("tags:render", this.handleRender);
    }

    render() {
        const rows = visibleStocks();
        if (this.hasTitleTarget) {
            this.titleTarget.textContent = state.untaggedSelected
                ? "Stocks: Untagged"
                : state.selectedTagIds.size
                    ? `Stocks: ${selectedTagTitle()}`
                    : "Stocks";
        }
        if (this.hasCountTarget) this.countTarget.textContent = `${rows.length}`;
        if (!this.hasListTarget) return;

        this.listTarget.innerHTML = rows.map(stock => `
            <div class="item ${stock.ticker === state.selectedTicker ? "selected" : ""}" data-ticker="${stock.ticker}" data-action="click->stock-list#selectStock">
                <div class="stock-row-main">
                    <span class="ticker">${stock.ticker}</span>
                    <span class="count">${stock.tags.length || "untagged"}</span>
                </div>
                <div class="stock-tags">
                    ${stock.tags.length
                        ? stock.tags.map(tag => `<span class="chip" data-tag-chip="${tag.id}"><span>${escapeHtml(tag.name)}</span></span>`).join("")
                        : '<span class="no-tags">No tags</span>'}
                </div>
            </div>
        `).join("") || '<div class="item"><span class="no-tags">No stocks</span></div>';
    }

    selectStock(event) {
        const chip = event.target.closest("[data-tag-chip]");
        if (chip) {
            state.untaggedSelected = false;
            state.selectedTagIds = new Set([Number(chip.dataset.tagChip)]);
        }
        const nextTicker = event.currentTarget.dataset.ticker;
        if (state.selectedTicker === nextTicker && !chip) {
            state.selectedTicker = null;
            clearStockTagEditor();
        } else {
            if (state.selectedTicker !== nextTicker) clearStockTagEditor();
            state.selectedTicker = nextTicker;
        }
        requestRender();
    }

    clearStockSearch() {
        if (!this.hasSearchTarget) return;
        this.searchTarget.value = "";
        this.render();
    }
}
