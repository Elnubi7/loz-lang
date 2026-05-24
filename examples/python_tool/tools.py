def analyze_text(payload):
    text = payload["text"]
    return {
        "length": len(text),
        "label": "ok",
    }
