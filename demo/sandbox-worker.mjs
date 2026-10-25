// Only this worker has access to the interpreter. Generated code receives no host APIs.
import {newQuickJSWASMModuleFromVariant} from 'https://esm.sh/quickjs-emscripten-core@0.31.0';
import variant from 'https://esm.sh/@jitl/quickjs-singlefile-browser-release-sync@0.31.0';
const ready=newQuickJSWASMModuleFromVariant(variant);
self.onmessage=async({data})=>{
  try {
    if(typeof data.source!=='string'||data.source.length>16000||!Array.isArray(data.expenses)||data.expenses.length>30) throw Error('Invalid sandbox request.');
    const QuickJS=await ready;const outputs=[];
    for(const expense of data.expenses) {
      const runtime=QuickJS.newRuntime();runtime.setMemoryLimit(16*1024*1024);runtime.setMaxStackSize(256*1024);
      const deadline=Date.now()+100;runtime.setInterruptHandler(()=>Date.now()>deadline);
      const context=runtime.newContext();
      try {
        const result=context.evalCode(`${data.source}\n;JSON.stringify(evaluateExpense(${JSON.stringify(expense)}));`);
        if(result.error){result.error.dispose();throw Error('Candidate threw, exhausted memory, or exceeded its execution budget.');}
        let value;try {value=context.getString(result.value);} finally {result.value.dispose();}
        if(value.length>4096)throw Error('Candidate output exceeds 4 KB.');
        outputs.push(JSON.parse(value));
      } finally {context.dispose();runtime.dispose();}
    }
    self.postMessage({outputs});
  } catch(error){self.postMessage({error:String(error.message||error).slice(0,500)});}
};
