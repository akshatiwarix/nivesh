import { render } from "solid-js/web";
import { App } from "./app/App";
import "./styles/index.css";

const root = document.getElementById("root");
if (!root) {
  throw new Error("Nivesh: #root element is missing from index.html");
}
render(() => <App />, root);
