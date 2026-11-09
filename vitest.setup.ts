import "@testing-library/jest-dom/vitest";

// jsdom does not lay out pages, so it has no scrollIntoView for the queue view to call.
Element.prototype.scrollIntoView = function scrollIntoView() {};
