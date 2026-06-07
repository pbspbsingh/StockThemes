import { Application } from "https://unpkg.com/@hotwired/stimulus@3.2.2/dist/stimulus.js";

import TickerPopupController from "./controllers/ticker_popup_controller.js";
import { popupApi } from "./popup_api.js";

window.Stimulus = Application.start();
window.StockThemesPopup = popupApi;
Stimulus.register("ticker-popup", TickerPopupController);
