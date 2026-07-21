import { expect, test } from "@playwright/test";
import path from "node:path";

test("imports ontology-bound JSON and opens it in the graph explorer", async ({ page }) => {
  await page.goto("/");
  await expect(page.getByRole("main")).toContainText("ContextHub");

  const navigation = page.getByRole("navigation", { name: "Main navigation" });
  await navigation.getByRole("button", { name: "Data mapping" }).click();
  await expect(page.getByRole("heading", { name: "Data import" })).toBeVisible();

  await page.locator('input[type="file"]').setInputFiles(path.join(__dirname, "fixtures/services.json"));
  await expect(page.getByRole("status")).toContainText(/3 (preview records loaded|records ready)/, { timeout: 30_000 });
  await expect(page.getByRole("code").filter({ hasText: "service_id" })).toBeVisible();

  await page.getByRole("button", { name: "Preview" }).click();
  await expect(page.getByRole("status")).toContainText("3 objects previewed");

  await page.getByRole("button", { name: "Import", exact: true }).click();
  await expect(page.getByRole("heading", { name: "services.json" })).toBeVisible({ timeout: 90_000 });
  await expect(page.locator(".explorer-header p")).toContainText("3 loaded objects");
  await expect(page.getByText(/Showing 3 of 3 loaded objects/)).toBeVisible();

  await page.getByPlaceholder("Search objects…").fill("e2e-api");
  await page.locator(".search-results button").first().click();
  await expect(page.locator(".floating-inspector .mono-id")).toContainText("service:e2e-api");

  await page.getByRole("button", { name: "Query", exact: true }).click();
  await expect(page.getByLabel("Graph query builder")).toBeVisible();
  await page.getByRole("button", { name: "Close query builder" }).click();
});
