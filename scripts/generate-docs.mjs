import { mkdir, readFile, writeFile } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const projectRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const checkOnly = process.argv.includes('--check');
const locales = [
  { key: 'zh-CN', suffix: 'zh-CN' },
  { key: 'en', suffix: 'en' },
];

async function readJson(relativePath) {
  const filePath = path.join(projectRoot, relativePath);
  try {
    return JSON.parse(await readFile(filePath, 'utf8'));
  } catch (error) {
    if (error.code === 'ENOENT') {
      throw new Error(
        relativePath + ' is missing. Add the versioned bilingual source with both zh-CN and en text.',
      );
    }
    throw error;
  }
}

function localized(value, field) {
  if (!value || typeof value !== 'object') {
    throw new Error('Missing bilingual text for ' + field);
  }
  for (const locale of locales) {
    if (typeof value[locale.key] !== 'string' || value[locale.key].trim() === '') {
      throw new Error('Missing ' + locale.key + ' text for ' + field);
    }
  }
  return value;
}

function renderReadme(productName, version, content, locale) {
  const lang = locale.key;
  const lines = [
    '# ' + productName,
    '',
    '[English](README.md) | [简体中文](README.zh-CN.md)',
    '',
    localized(content.tagline, 'README tagline')[lang],
  ];

  if (!Array.isArray(content.sections) || content.sections.length === 0) {
    throw new Error('README must contain at least one section.');
  }

  for (const [sectionIndex, section] of content.sections.entries()) {
    const sectionTitle = localized(section.title, 'README section ' + sectionIndex + ' title')[lang];
    if (!Array.isArray(section.items) || section.items.length === 0) {
      throw new Error('README section ' + sectionIndex + ' must contain at least one item.');
    }
    lines.push('', '## ' + sectionTitle);
    for (const [itemIndex, item] of section.items.entries()) {
      lines.push('- ' + localized(item, 'README section ' + sectionIndex + ' item ' + itemIndex)[lang]);
    }
  }

  if (content.markdownSections !== undefined) {
    if (!Array.isArray(content.markdownSections)) {
      throw new Error('README markdownSections must be an array.');
    }
    for (const [sectionIndex, section] of content.markdownSections.entries()) {
      const sectionTitle = localized(section.title, 'README markdown section ' + sectionIndex + ' title')[lang];
      const markdown = localized(section.content, 'README markdown section ' + sectionIndex + ' content')[lang].trim();
      if (!markdown) {
        throw new Error('README markdown section ' + sectionIndex + ' must contain content.');
      }
      lines.push('', '## ' + sectionTitle, '', ...markdown.split(/\r?\n/));
    }
  }

  const notesName = 'v' + version + '.' + locale.suffix + '.md';
  const notesLabel = lang === 'zh-CN' ? '本版本发布说明' : 'Release notes for this version';
  const latestLabel = lang === 'zh-CN' ? '下载最新版本' : 'Download the latest release';
  lines.push(
    '',
    '## ' + (lang === 'zh-CN' ? '发布与下载' : 'Releases and downloads'),
    '- [' + notesLabel + '](docs/releases/' + notesName + ')',
    '- [' + latestLabel + '](https://github.com/mrshuangok/CQBar/releases/latest)',
    '',
  );
  return lines.join('\n');
}

function renderReleaseNotes(productName, version, source, locale) {
  const lang = locale.key;
  const title = localized(source.title, 'release title')[lang];
  const summary = localized(source.summary, 'release summary')[lang];
  if (!/^\d{4}-\d{2}-\d{2}$/.test(source.date || '')) {
    throw new Error('Release notes date must use YYYY-MM-DD.');
  }
  if (!Array.isArray(source.highlights) || source.highlights.length === 0) {
    throw new Error('Release notes must contain at least one highlight.');
  }

  const lines = [
    '# ' + productName + ' v' + version + ' — ' + title,
    '',
    (lang === 'zh-CN' ? '发布日期：' : 'Release date: ') + source.date,
    '',
    summary,
    '',
    '## ' + (lang === 'zh-CN' ? '更新内容' : 'Highlights'),
  ];
  for (const [index, item] of source.highlights.entries()) {
    lines.push('- ' + localized(item, 'release highlight ' + index)[lang]);
  }
  lines.push('');
  return lines.join('\n');
}

async function main() {
  const config = await readJson('src-tauri/tauri.conf.json');
  if (typeof config.version !== 'string' || typeof config.productName !== 'string') {
    throw new Error('Product name and version are required in tauri.conf.json.');
  }
  const content = await readJson('docs/readme-content.json');
  const releasePath = 'docs/release-notes/' + config.version + '.json';
  const release = await readJson(releasePath);
  if (release.version !== config.version) {
    throw new Error('Release note source version must match tauri.conf.json: ' + config.version);
  }

  const outputs = [];
  for (const locale of locales) {
    const readmePath = locale.key === 'zh-CN' ? 'README.zh-CN.md' : 'README.md';
    const notesPath = 'docs/releases/v' + config.version + '.' + locale.suffix + '.md';
    outputs.push(
      [readmePath, renderReadme(config.productName, config.version, content, locale)],
      [notesPath, renderReleaseNotes(config.productName, config.version, release, locale)],
    );
  }

  for (const [relativePath, expected] of outputs) {
    const filePath = path.join(projectRoot, relativePath);
    if (checkOnly) {
      let actual;
      try {
        actual = await readFile(filePath, 'utf8');
      } catch {
        throw new Error(relativePath + ' is missing; run npm run docs:generate.');
      }
      if (actual !== expected) {
        throw new Error(relativePath + ' is out of date; run npm run docs:generate.');
      }
    } else {
      await mkdir(path.dirname(filePath), { recursive: true });
      await writeFile(filePath, expected, 'utf8');
      console.log('Generated ' + relativePath);
    }
  }
  if (checkOnly) {
    console.log('Bilingual README and release notes are up to date.');
  }
}

main().catch((error) => {
  console.error(error.message);
  process.exitCode = 1;
});
