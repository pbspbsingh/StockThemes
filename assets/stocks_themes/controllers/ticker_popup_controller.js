import { Controller } from "https://unpkg.com/@hotwired/stimulus@3.2.2/dist/stimulus.js";

import { popupApi } from "../popup_api.js";
import { PopupCharts } from "../popup_charts.js";
import { RrgChart } from "../rrg_chart.js";

export default class extends Controller {
    static targets = [
        "backdrop",
        "title",
        "chartsTab",
        "rrgTab",
        "chartsPanel",
        "rrgPanel",
        "rrgTickerToggle",
        "sectorLabel",
        "industryLabel",
        "industryPanel",
        "sectorContainer",
        "industryContainer",
        "rrgBody",
        "rrgCanvas",
    ];

    connect() {
        this.info = null;
        this.activeTabName = "charts";
        this.popupCharts = new PopupCharts({
            sectorLabel: this.sectorLabelTarget,
            industryLabel: this.industryLabelTarget,
            industryPanel: this.industryPanelTarget,
            sectorContainer: this.sectorContainerTarget,
            industryContainer: this.industryContainerTarget,
        });
        this.rrgChart = new RrgChart({
            body: this.rrgBodyTarget,
            canvas: this.rrgCanvasTarget,
            tickerToggle: this.rrgTickerToggleTarget,
            isCurrent: info =>
                this.isOpen() &&
                this.activeTab() === "rrg" &&
                this.info?.ticker === info.ticker,
        });
        popupApi.register(this);
    }

    disconnect() {
        this.close();
        popupApi.unregister(this);
    }

    open(info, tab = "charts") {
        if (!info) return;

        this.info = info;
        this.titleTarget.innerHTML =
            `<span style="color:#4a9eff;font-weight:600">${this.escapeHtml(info.ticker)}</span>` +
            '<span style="color:#555;font-weight:400;font-size:11px;margin-left:6px">Sector &amp; Industry / RRG</span>';
        this.backdropTarget.classList.add("open");
        document.body.style.overflow = "hidden";
        this.switchTo(tab);
    }

    switchTab(event) {
        this.switchTo(event.currentTarget.dataset.tab);
    }

    switchTo(tab) {
        const nextTab = tab === "rrg" ? "rrg" : "charts";
        const isRrg = nextTab === "rrg";
        this.activeTabName = nextTab;

        this.chartsTabTarget.classList.toggle("active", !isRrg);
        this.rrgTabTarget.classList.toggle("active", isRrg);
        this.chartsPanelTarget.classList.toggle("active", !isRrg);
        this.rrgPanelTarget.classList.toggle("active", isRrg);
        this.rrgTickerToggleTarget.style.display = isRrg ? "" : "none";

        if (!this.info) return;

        if (isRrg) {
            this.popupCharts.destroy();
            this.rrgChart.render(this.info);
        } else {
            this.rrgChart.cancel();
            this.popupCharts.render(this.info);
        }
    }

    activeTab() {
        return this.activeTabName;
    }

    isOpen() {
        return this.backdropTarget.classList.contains("open");
    }

    closeFromBackdrop(event) {
        if (event.target === this.backdropTarget) this.close();
    }

    closeOnEscape(event) {
        if (event.key === "Escape") this.close();
    }

    close() {
        if (!this.isOpen()) return;

        this.backdropTarget.classList.remove("open");
        document.body.style.overflow = "";
        this.popupCharts.destroy();
        this.rrgChart.cancel();
        this.info = null;
    }

    toggleRrgTicker() {
        if (this.info) this.rrgChart.toggleTicker(this.info);
    }

    escapeHtml(value) {
        const node = document.createElement("div");
        node.textContent = String(value ?? "");
        return node.innerHTML;
    }
}
