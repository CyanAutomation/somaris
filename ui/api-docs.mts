type HttpMethod = 'get' | 'post' | 'put' | 'patch' | 'delete';

interface OpenApiOperation {
  summary?: string;
  description?: string;
}

type OpenApiPathItem = Partial<Record<HttpMethod, OpenApiOperation>>;

export interface OpenApiDocument {
  info: {title: string};
  paths: Record<string, OpenApiPathItem>;
}

const methods = ['get', 'post', 'put', 'patch', 'delete'] as const satisfies readonly HttpMethod[];

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

function isOperation(value: unknown): value is OpenApiOperation {
  return isRecord(value)
    && (value.summary === undefined || typeof value.summary === 'string')
    && (value.description === undefined || typeof value.description === 'string');
}

export function parseOpenApiDocument(value: unknown): OpenApiDocument {
  if (!isRecord(value) || !isRecord(value.info) || typeof value.info.title !== 'string' || !isRecord(value.paths)) {
    throw new Error('The OpenAPI document has an unexpected format.');
  }

  const paths: Record<string, OpenApiPathItem> = {};
  for (const [path, pathValue] of Object.entries(value.paths)) {
    if (!isRecord(pathValue)) continue;
    const pathItem: OpenApiPathItem = {};
    for (const method of methods) {
      const operation = pathValue[method];
      if (operation !== undefined && isOperation(operation)) pathItem[method] = operation;
    }
    paths[path] = pathItem;
  }

  return {info: {title: value.info.title}, paths};
}

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

export async function loadOpenApiReference(root: Document = document): Promise<void> {
  const heading = root.querySelector<HTMLHeadingElement>('h1');
  const operations = root.querySelector<HTMLElement>('#operations');
  const status = root.querySelector<HTMLElement>('#status');
  if (!heading || !operations || !status) return;

  try {
    const response = await fetch('/openapi.json');
    if (!response.ok) throw new Error(`HTTP ${response.status}`);
    const spec = parseOpenApiDocument(await response.json() as unknown);
    root.title = `${spec.info.title} reference`;
    heading.textContent = `${spec.info.title} reference`;

    for (const [path, pathItem] of Object.entries(spec.paths)) {
      for (const method of methods) {
        const operation = pathItem[method];
        if (!operation) continue;
        const row = root.createElement('section');
        row.className = 'operation';
        const headingRow = root.createElement('div');
        const verb = root.createElement('span');
        verb.className = 'method';
        verb.textContent = method.toUpperCase();
        const route = root.createElement('code');
        route.textContent = path;
        headingRow.append(verb, route);
        row.append(headingRow);
        const descriptionText = operation.summary || operation.description;
        if (descriptionText) {
          const description = root.createElement('p');
          description.className = 'description';
          description.textContent = descriptionText;
          row.append(description);
        }
        operations.append(row);
      }
    }
  } catch (error: unknown) {
    status.textContent = `Could not load the OpenAPI document: ${errorMessage(error)}`;
  }
}

if (typeof document !== 'undefined') void loadOpenApiReference(document);
