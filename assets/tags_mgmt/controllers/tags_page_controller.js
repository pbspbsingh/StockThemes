import { Controller } from "https://unpkg.com/@hotwired/stimulus@3.2.2/dist/stimulus.js";

import {
    applyInitialTickerSelection,
    requestRender,
    scrollSelectedTickerIntoView,
    setData,
    state,
} from "../state.js";

export default class extends Controller {
    static targets = ["toast"];
    static values = {
        tagSuggestionEnabled: Boolean,
    };

    connect() {
        this.toastTimer = null;
        this.handleStatus = event => this.setStatus(event.detail.message, event.detail.type);
        window.addEventListener("tags:status", this.handleStatus);

        setData({
            tags: this.parseJson("tags-data"),
            categories: this.parseJson("categories-data"),
            stocks: this.parseJson("stocks-data"),
            untagged: this.parseJson("untagged-data"),
        });
        state.tagSuggestionEnabled = this.tagSuggestionEnabledValue;
        applyInitialTickerSelection();
        requestRender();
        scrollSelectedTickerIntoView();
    }

    disconnect() {
        window.removeEventListener("tags:status", this.handleStatus);
        if (this.toastTimer) clearTimeout(this.toastTimer);
    }

    parseJson(id) {
        const node = document.getElementById(id);
        if (!node) return [];
        return JSON.parse(node.textContent);
    }

    setStatus(message, type = "") {
        if (!this.hasToastTarget) return;
        if (this.toastTimer) clearTimeout(this.toastTimer);

        this.toastTarget.textContent = message || "";
        this.toastTarget.className = "toast" + (type ? " " + type : "");
        if (!message) return;

        requestAnimationFrame(() => this.toastTarget.classList.add("show"));
        this.toastTimer = setTimeout(() => {
            this.toastTarget.classList.remove("show");
            this.toastTimer = null;
        }, type === "error" ? 5000 : 3000);
    }
}
