export function createLazyModule<T>(load:()=>Promise<T>):{
 loaded:T|undefined;
 load:()=>Promise<T>;
} {
 let loaded:T|undefined;
 let pending:Promise<T>|undefined;
 return {
  get loaded(){return loaded;},
  load(){
   if(loaded!==undefined)return Promise.resolve(loaded);
   if(!pending){
    pending=load().then(module=>{
     loaded=module;
     return module;
    }).finally(()=>{pending=undefined;});
   }
   return pending;
  },
 };
}
