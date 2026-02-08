from app.services.llm_service import llm_service

print("Ayla:", end=" ", flush=True)

for chunk in llm_service.generate_stream("Oi Ayla, quem é você?"):
    print(chunk, end="", flush=True)

print()
