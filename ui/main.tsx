import { render } from "solid-js/web";
import { App } from "./app/App";
import "./styles/index.css";

const root = document.getElementById("root");
if (!root) {
  throw new Error("Nivesh: #root element is missing from index.html");
}
// Nothing in the app may navigate the webview to a web page. Links (such as the
// chart library's attribution) do nothing; headline links go through the
// backend, which opens an allowlisted http(s) link in the system browser.
document.addEventListener("click", (e) => {
  if (e.target instanceof Element && e.target.closest("a[href]")) e.preventDefault();
});

render(() => <App />, root);
