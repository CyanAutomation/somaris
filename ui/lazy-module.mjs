export function createLazyModule(load) {
    let loaded;
    let pending;
    return {
        get loaded() { return loaded; },
        load() {
            if (loaded !== undefined)
                return Promise.resolve(loaded);
            if (!pending) {
                pending = load().then(module => {
                    loaded = module;
                    return module;
                }).finally(() => { pending = undefined; });
            }
            return pending;
        },
    };
}
