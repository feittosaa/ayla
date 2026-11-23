// A Ayla abre PDF no próprio leitor (pdf.js já embutido na interface); o foliate-js só é usado
// para EPUB, e este módulo substitui o original para não trazer outra cópia do pdf.js.
export const makePDF = async () => { throw new Error('PDF abre no leitor de PDF da Ayla') }
