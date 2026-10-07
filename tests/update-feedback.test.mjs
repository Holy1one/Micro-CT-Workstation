/** Renders the actual feedback component to distinguish success from failure. */
import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { loadSourceModule } from './load-source-module.mjs';
const { UpdateFeedback } = await loadSourceModule('src/UpdateDialog.tsx');
test('successful receipt notice renders as status without error styling or alert', () => {
  const markup = renderToStaticMarkup(createElement(UpdateFeedback, { notice: '已核对更新完成：0.9.0-rc.2', error: null }));
  assert.match(markup, /role="status"/);
  assert.match(markup, /class="update-notice"/);
  assert.doesNotMatch(markup, /role="alert"|update-error/);
  assert.match(markup, /已核对更新完成/);
});
test('failed or incomplete receipt remains an explicit alert with its actual reason', () => {
  for (const error of ['上次更新失败：1603', '上次更新未确认完成']) {
    const markup = renderToStaticMarkup(createElement(UpdateFeedback, { notice: null, error }));
    assert.match(markup, /role="alert"/); assert.match(markup, /class="update-error"/);
    assert.doesNotMatch(markup, /update-notice/); assert.ok(markup.includes(error));
  }
});
test('cleared notice and error render no residual completion or failure message', () => {
  assert.equal(renderToStaticMarkup(createElement(UpdateFeedback, { notice: null, error: null })), '');
});
